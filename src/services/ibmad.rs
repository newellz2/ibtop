use super::lib::{CounterEvent, CountersService, DiscoverService, DiscoveryEvent, Node};
use crate::{
    app::AppConfig,
    services::lib::{LidPort, Port},
};
use chrono::Utc;
use rayon::{ThreadPoolBuilder, prelude::*};
use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender},
};

pub struct IbmadDiscoveryService {
    ev_disc_rx: Receiver<DiscoveryEvent>,
    disc_ev_tx: Sender<DiscoveryEvent>,
    config: AppConfig,
}

impl IbmadDiscoveryService {
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
                        return Ok(());
                    }
                    DiscoveryEvent::Request => {
                        let nodes = self.get_nodes();
                        if let Err(e) = self.disc_ev_tx.send(DiscoveryEvent::Response(nodes)) {
                            eprintln!("Failed to send discovery response: {e}");
                        }
                    }
                    _ => {
                        eprintln!("Received unexpected DiscoveryEvent: {ev:?}");
                    }
                },
                Err(e) => {
                    eprintln!("DiscoveryService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl DiscoverService for IbmadDiscoveryService {
    fn get_nodes(&self) -> Vec<Node> {
        let hca_name = &self.config.hca;
        let ca = match ibmad::ca::get_ca(hca_name) {
            Ok(ca) => ca,
            Err(e) => {
                eprintln!("Failed to open CA {}: {}", hca_name, e);
                return Vec::new();
            }
        };

        let port = match ibmad::mad::open_smp_port(&ca) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to open SMP port for CA {}: {}", hca_name, e);
                return Vec::new();
            }
        };

        return self.perform_discovery(port);
    }
}

impl IbmadDiscoveryService {
    fn perform_discovery(&self, mut port: ibmad::mad::IbMadPort) -> Vec<Node> {
        let agent_id = match ibmad::mad::register_agent(
            &mut port,
            ibmad::mad::IB_MGMT_CLASS_DIRECT_ROUTED_SMP,
        ) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Failed to register agent: {}", e);
                return Vec::new();
            }
        };

        let mut fabric = ibmad::discovery::Fabric {
            port,
            agent_id,
            node_map: HashMap::new(),
            nodes: Vec::new(),
            switches: Vec::new(),
            hcas: Vec::new(),
            dr_paths: HashMap::new(),
            ni_timings: Vec::new(),
            retries: self.config.discovery_retries,
            timeout: self.config.discovery_timeout,
            mad_errors: 0,
            mad_timeouts: 0,
            mads_sent: 0,
            tid: 0,
        };

        if let Err(e) = fabric.seq_discover() {
            eprintln!("Discovery failed: {}", e);
            return Vec::new();
        }

        let mut nodes = Vec::new();

        // Convert fabric nodes to ibtop nodes
        for node_arc in fabric.nodes {
            let node = match node_arc.read() {
                Ok(n) => n,
                Err(_) => continue,
            };

            let mut ports = Vec::new();
            for port_arc in &node.ports {
                let p = match port_arc.read() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let remote_node_desc = if let Some(remote_weak) = &p.remote_port {
                    if let Some(remote_lock) = remote_weak.upgrade() {
                        if let Ok(remote_port) = remote_lock.read() {
                            let parent_weak = &remote_port.parent;
                            if let Some(parent_lock) = parent_weak.upgrade() {
                                if let Ok(parent_node) = parent_lock.read() {
                                    parent_node.description.clone().unwrap_or_default()
                                } else {
                                    String::new()
                                }
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                ports.push(Port {
                    number: p.number as i32,
                    remote_node_description: remote_node_desc,
                });
            }

            if node.node_type == ibmad::enums::IbNodeType::Switch {
                nodes.push(Node {
                    guid: node.node_guid,
                    node_description: node.description.clone().unwrap_or_default(),
                    ports,
                    lid: node.lid,
                });
            } else if node.node_type == ibmad::enums::IbNodeType::CA && self.config.include_hcas {
                nodes.push(Node {
                    guid: node.node_guid,
                    node_description: node.description.clone().unwrap_or_default(),
                    ports,
                    lid: node.lid,
                });
            }
        }

        nodes
    }
}

pub struct IbmadCountersService {
    ev_ctr_rx: Receiver<CounterEvent>,
    ctr_ev_tx: Sender<CounterEvent>,
    config: AppConfig,
}

impl IbmadCountersService {
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
                    CounterEvent::Exit => return Ok(()),
                    CounterEvent::Request(nodes) => {
                        let counters = self.get_counters(nodes);
                        if let Err(e) = self.ctr_ev_tx.send(CounterEvent::Response(counters)) {
                            eprintln!("Failed to send counters response: {e}");
                        }
                    }
                    _ => eprintln!("Received unexpected CounterEvent: {ev:?}"),
                },
                Err(e) => {
                    eprintln!("CountersService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl CountersService for IbmadCountersService {
    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>> {
        let hca_name = self.config.hca.clone();
        let timeout = self.config.update_timeout;
        let retries = self.config.update_retries;
        let pkey = self.config.pkey;

        let pool = match ThreadPoolBuilder::new()
            .num_threads(self.config.threads)
            .build()
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create thread pool: {e}");
                return HashMap::new();
            }
        };

        pool.install(|| {
            let num_threads = rayon::current_num_threads().max(1);
            let chunk_size = (lid_ports.len() / num_threads).max(1);

            lid_ports
                .par_chunks(chunk_size)
                .map(|chunk| {
                    let mut local_map = HashMap::new();

                    let ca = match ibmad::ca::get_ca(&hca_name) {
                        Ok(ca) => ca,
                        Err(e) => {
                            eprintln!("Failed to open CA: {e}");
                            return local_map;
                        }
                    };

                    let mut port = match ibmad::mad::open_port(&ca) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("Failed to open MAD port: {e}");
                            return local_map;
                        }
                    };

                    let agent_id = match ibmad::mad::register_agent(
                        &mut port,
                        ibmad::mad::IB_MGMT_CLASS_PERFORMANCE,
                    ) {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("Failed to register PerfMgt agent: {e}");
                            return local_map;
                        }
                    };

                    for lp in chunk {
                        let start = Utc::now();
                        let res = ibmad::mad::query_port_counters_extended(
                            &mut port,
                            agent_id,
                            timeout,
                            retries,
                            lp.lid,
                            lp.number as u8,
                            pkey as u16,
                        );
                        let end = Utc::now();

                        let mut counters = HashMap::new();

                        match res {
                            Ok(perf) => {
                                counters.insert(
                                    "start_timestamp".to_string(),
                                    start.timestamp_nanos_opt().unwrap_or(0) as u64,
                                );
                                counters.insert(
                                    "end_timestamp".to_string(),
                                    end.timestamp_nanos_opt().unwrap_or(0) as u64,
                                );

                                counters.insert(
                                    "symbol_errors".to_string(),
                                    perf.symbol_error_counter(),
                                );
                                counters.insert(
                                    "link_recovers".to_string(),
                                    perf.link_error_recovery_counter(),
                                );
                                counters
                                    .insert("link_downed".to_string(), perf.link_downed_counter());
                                counters.insert("rcv_errors".to_string(), perf.port_rcv_errors());
                                counters.insert(
                                    "phys_rcv_errors".to_string(),
                                    perf.port_rcv_remote_physical_errors(),
                                );
                                counters.insert(
                                    "switch_rel_errors".to_string(),
                                    perf.port_rcv_switch_relay_errors(),
                                );
                                counters.insert(
                                    "excess_overrun_errors".to_string(),
                                    perf.excessive_buffer_overrun_errors(),
                                );
                                counters.insert("vl15dropped".to_string(), perf.vl15_dropped());
                                counters.insert("qp1_drops".to_string(), perf.qp1_dropped());

                                counters
                                    .insert("port_xmit_data".to_string(), perf.port_xmit_data());
                                counters.insert("port_rcv_data".to_string(), perf.port_rcv_data());
                                counters
                                    .insert("port_xmit_wait".to_string(), perf.port_xmit_wait());
                                counters
                                    .insert("port_xmit_pkts".to_string(), perf.port_xmit_pkts());
                                counters.insert("port_rcv_pkts".to_string(), perf.port_rcv_pkts());

                                local_map.insert((lp.lid, lp.number), counters);
                            }
                            Err(e) => {
                                eprintln!(
                                    "Failed to query counters for LID {} Port {}: {}",
                                    lp.lid, lp.number, e
                                );
                                counters.insert("error".to_string(), 1);
                                local_map.insert((lp.lid, lp.number), counters);
                            }
                        }
                    }

                    local_map
                })
                .reduce(HashMap::new, |mut a, b| {
                    a.extend(b);
                    a
                })
        })
    }
}
