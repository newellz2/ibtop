use crate::{app::AppConfig, services::lib::{LidPort, Port}};
use chrono::Utc;
use rayon::{prelude::*, ThreadPoolBuilder};
use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender},
};
use super::lib::{CounterEvent, CountersService, DiscoverService, DiscoveryEvent, Node};

pub const ERROR_COUNTERS: [&str; 11] = [
    "symbol_errors",
    "link_recovers",
    "link_downed",
    "rcv_errors",
    "phys_rcv_errors",
    "switch_rel_errors",
    "rcv_local_phy_errors",
    "rcv_malformed_pkt_errors",
    "excess_overrun_errors",
    "vl15dropped",
    "qp1_drops",
];

pub struct RsmadDiscoveryService {
    ev_disc_rx: Receiver<DiscoveryEvent>,
    disc_ev_tx: Sender<DiscoveryEvent>,
    config: AppConfig,
}

impl RsmadDiscoveryService {
    pub fn new(
        ev_disc_rx: Receiver<DiscoveryEvent>,
        disc_ev_tx: Sender<DiscoveryEvent>,
        config: AppConfig,
    ) -> Self {
        Self {
            ev_disc_rx,
            disc_ev_tx,
            config,
        }
    }

    pub fn run(self) -> color_eyre::Result<()> {
        loop {
            match self.ev_disc_rx.recv() {
                Ok(ev) => match ev {
                    DiscoveryEvent::Exit => {
                        // Terminate thread
                        return Ok(());
                    }
                    DiscoveryEvent::Request => {
                        let nodes = self.get_nodes();
                        // Send the response even if empty
                        let _ = self.disc_ev_tx.send(DiscoveryEvent::Response(nodes));
                    }
                    // Possibly log unknown events
                    _ => {
                        eprintln!("Received unexpected DiscoveryEvent: {:?}", ev);
                    }
                },
                // If the sender is gone, we can exit or continue 
                Err(e) => {
                    eprintln!("DiscoveryService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl DiscoverService for RsmadDiscoveryService {
    fn get_nodes(&self) -> Vec<Node> {
        rsmad::umad::umad_init();
        unsafe { rsmad::ibmad::sys::madrpc_show_errors(0) };

        let mut nodes = Vec::new();
        let mut fabric = rsmad::ibnetdisc::fabric::Fabric::new(&self.config.hca);
        let discover_res = fabric.discover(
            1,
            self.config.timeout,
            self.config.retries, 
            0, 0, 0, 0,
        );

        if let Err(e) = discover_res {
            eprintln!("Error discovering fabric: {e}");
            // Return the empty Vec or partial data
            rsmad::umad::umad_done();
            return nodes;
        }

        // If success, parse the discovered nodes
        for (_guid, rc_node) in fabric.nodes {
            let nd_ref = rc_node.borrow();
            match nd_ref.node_type {
                rsmad::ibnetdisc::node::NodeType::CA => {
                    if self.config.include_hcas {
                        nodes.push(Node {
                            guid: nd_ref.guid,
                            node_description: nd_ref.node_desc.clone(),
                            ports: vec![],
                            lid: nd_ref.lid,
                        });
                        }
                }
                rsmad::ibnetdisc::node::NodeType::SWITCH => {

                    let ports: Vec<Port> = vec![];

                    nodes.push(Node {
                        guid: nd_ref.guid,
                        node_description: nd_ref.node_desc.clone(),
                        ports: ports,
                        lid: nd_ref.lid,
                    });
                }
                _ => {}
            }
        }

        rsmad::umad::umad_done();
        nodes
    }
}

// Counters service
pub struct RsmadCountersService {
    ev_ctr_rx: Receiver<CounterEvent>,
    ctr_ev_tx: Sender<CounterEvent>,
    config: AppConfig,
}

impl RsmadCountersService {
    pub fn new(
        ev_ctr_rx: Receiver<CounterEvent>,
        ctr_ev_tx: Sender<CounterEvent>,
        config: AppConfig,
    ) -> Self {
        Self {
            ev_ctr_rx,
            ctr_ev_tx,
            config,
        }
    }

    pub fn run(self) -> color_eyre::Result<()> {
        loop {
            match self.ev_ctr_rx.recv() {
                Ok(ev) => match ev {
                    CounterEvent::Exit => {
                        return Ok(());
                    }
                    CounterEvent::Request(nodes) => {
                        let counters = self.get_counters(nodes);
                        let _ = self.ctr_ev_tx.send(CounterEvent::Response(counters));
                    }
                    _ => {}
                },
                Err(_e) => {
                    return Ok(());
                }
            }
        }
    }
}

impl CountersService for RsmadCountersService {
    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>> {
        rsmad::umad::umad_init();
        unsafe {
            rsmad::ibmad::sys::madrpc_show_errors(0);
            rsmad::ibmad::sys::umad_debug(0);
        }

        let mgmt_classes = [rsmad::ibmad::sys::MAD_CLASSES_IB_PERFORMANCE_CLASS];
        let hca = self.config.hca.clone();
        let timeout = self.config.timeout;

        // Build thread pool
        let pool = match ThreadPoolBuilder::new()
            .num_threads(self.config.threads)
            .build()
        {
            Ok(p) => p,
            Err(_) => {
                return HashMap::new();
            }
        };

        let counters: HashMap<(u16, i32), HashMap<String, u64>> = pool.install(|| {
            lid_ports
                .par_iter()
                .filter_map(|lp| {
                    // Each iteration attempts to open a port
                    let port_result = rsmad::ibmad::mad_rpc_open_port(
                        &hca, 
                        &mgmt_classes
                    );

                    let mut port = match port_result {
                        Ok(p) => p,
                        Err(_) => {
                            return None;
                        }
                    };

                    let start = Utc::now();
                    let perfquery_res =
                        rsmad::ibmad::perfquery(&port, lp.lid.into(), lp.number, 0, timeout);
                    let end = Utc::now();

                    let result = match perfquery_res {
                        Ok(mut perfctrs) => {
                            perfctrs.counters.insert(
                                "start_timestamp".to_string(),
                                start.timestamp_nanos_opt().unwrap_or(0) as u64,
                            );
                            perfctrs.counters.insert(
                                "end_timestamp".to_string(),
                                end.timestamp_nanos_opt().unwrap_or(0) as u64,
                            );
                            Some(((lp.lid, lp.number), perfctrs.counters))
                        }
                        Err(_) => {
                            None
                        }
                    };

                    let _ = rsmad::ibmad::mad_rpc_close_port(&mut port);

                    result
                })
                .collect()
        });

        rsmad::umad::umad_done();
        counters
    }
}
