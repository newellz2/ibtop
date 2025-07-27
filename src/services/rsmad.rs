use crate::{app::AppConfig, services::lib::{LidPort, Port}};
use chrono::Utc;
use rayon::{prelude::*, ThreadPoolBuilder};
use std::{
    cell::RefCell, collections::HashMap, sync::mpsc::{Receiver, Sender}
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
                        if let Err(e) = self.disc_ev_tx.send(DiscoveryEvent::Response(nodes)) {
                            eprintln!("Failed to send discovery response: {e}");
                        }
                    }
                    // Log unknown events for debugging
                    _ => {
                        eprintln!("Received unexpected DiscoveryEvent: {ev:?}");
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
        let init_result = rsmad::umad::umad_init();
        if init_result != 0 {
            eprintln!("Failed to initialize UMAD: error code {}", init_result);
            return Vec::new();
        }
        
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

        // Single pass, immediate processing
        let mut port_connections: HashMap<(u64, i32), String> = HashMap::new();
        let mut pending_nodes: Vec<(u64, String, u16, rsmad::ibnetdisc::node::NodeType, Option<Vec<i32>>)> = Vec::new();

        // Extract all connection info AND basic node data in one pass
        for (_guid, rc_node) in &fabric.nodes {
            let nd_ref = rc_node.borrow();
            
            // Store basic node info for second pass
            let port_numbers = nd_ref.ports.as_ref().map(|ports| {
                ports.iter().map(|p| p.as_ref().borrow().number).collect()
            });
            
            pending_nodes.push((
                nd_ref.guid,
                nd_ref.node_desc.clone(),
                nd_ref.lid,
                nd_ref.node_type.clone(),
                port_numbers
            ));

            // Extract connection info while weak refs are valid
            if let Some(ports) = &nd_ref.ports {
                for p in ports {
                    let p_ref = p.as_ref().borrow();
                    if let (Some(weak_remote_port), Some(weak_remote_node)) =
                        (&p_ref.remote_port, &p_ref.remote_node)
                    {
                        if let (Some(_remote_port), Some(remote_node)) =
                            (weak_remote_port.upgrade(), weak_remote_node.upgrade())
                        {
                            let remote_node_ref = RefCell::borrow(&remote_node);
                            port_connections.insert(
                                (nd_ref.guid, p_ref.number),
                                remote_node_ref.node_desc.clone()
                            );
                        }
                    }
                }
            }
        }

        // Process stored node data, drop fabric
        drop(fabric);
        
        for (guid, node_desc, lid, node_type, port_numbers) in pending_nodes {
            match node_type {
                rsmad::ibnetdisc::node::NodeType::CA => {
                    if self.config.include_hcas {
                        nodes.push(Node {
                            guid,
                            node_description: node_desc,
                            ports: Vec::new(),
                            lid,
                        });
                    }
                }
                rsmad::ibnetdisc::node::NodeType::SWITCH => {
                    let ports = if let Some(port_nums) = port_numbers {
                        port_nums.into_iter().map(|port_num| {
                            let remote_desc = port_connections
                                .get(&(guid, port_num))
                                .cloned()
                                .unwrap_or_default();

                            Port { 
                                number: port_num,
                                remote_node_description: remote_desc,
                            }
                        }).collect()
                    } else {
                        Vec::new()
                    };

                    nodes.push(Node {
                        guid,
                        node_description: node_desc,
                        ports,
                        lid,
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
                        if let Err(e) = self.ctr_ev_tx.send(CounterEvent::Response(counters)) {
                            eprintln!("Failed to send counters response: {e}");
                        }
                    }
                    _ => {
                        eprintln!("Received unexpected CounterEvent: {ev:?}");
                    }
                },
                Err(e) => {
                    eprintln!("CountersService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl CountersService for RsmadCountersService {
    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>> {
        // Initialize UMAD
        let init_result = rsmad::umad::umad_init();
        if init_result != 0 {
            eprintln!("Failed to initialize UMAD: error code {}", init_result);
            return HashMap::new();
        }
        
        // Set error reporting and debug levels
        unsafe {
            rsmad::ibmad::sys::madrpc_show_errors(0);
            rsmad::ibmad::sys::umad_debug(0);
        }

        let mgmt_classes = [rsmad::ibmad::sys::MAD_CLASSES_IB_PERFORMANCE_CLASS];
        let hca = self.config.hca.clone();
        let timeout = self.config.timeout;

        // Build thread pool with error handling
        let pool = match ThreadPoolBuilder::new()
            .num_threads(self.config.threads)
            .build()
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create thread pool: {e}");
                rsmad::umad::umad_done();
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
                        Err(e) => {
                            eprintln!("Failed to open port for LID {}: {e}", lp.lid);
                            return None;
                        }
                    };

                    let start = Utc::now();
                    let perfquery_res =
                        rsmad::ibmad::perfquery(&port, lp.lid.into(), lp.number, 0, timeout);
                    let end = Utc::now();

                    let result = match perfquery_res {
                        Ok(mut perfctrs) => {
                            // Add timestamps for bandwidth calculations
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
                        Err(e) => {
                            eprintln!("Failed to query performance counters for LID {} port {}: {e}", lp.lid, lp.number);
                            None
                        }
                    };

                    // Always close the port
                    if let Err(e) = rsmad::ibmad::mad_rpc_close_port(&mut port) {
                        eprintln!("Failed to close port for LID {}: {e}", lp.lid);
                    }

                    result
                })
                .collect()
        });

        rsmad::umad::umad_done();
        counters
    }
}
