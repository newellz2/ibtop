use crate::{
    app::AppConfig,
    services::lib::{
        CounterEvent, CountersService, DiscoverService, DiscoveryEvent, LidPort, Node, Port,
    },
};
use chrono::Utc;
use ibmad::{ca, discovery, enums, mad};
use rayon::{ThreadPoolBuilder, prelude::*};
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender},
    },
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
                    other => {
                        eprintln!("Received unexpected DiscoveryEvent: {other:?}");
                    }
                },
                Err(e) => {
                    eprintln!("DiscoveryService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }

    fn convert_ports(node_arc: &Arc<RwLock<discovery::Node>>) -> Vec<Port> {
        let mut ports = Vec::new();
        let Ok(node_guard) = node_arc.read() else {
            return ports;
        };

        for port_arc in &node_guard.ports {
            let port_guard = match port_arc.read() {
                Ok(guard) => guard,
                Err(_) => continue,
            };

            let remote_desc = port_guard
                .remote_port
                .as_ref()
                .and_then(|weak| weak.upgrade())
                .and_then(|remote_arc| {
                    remote_arc.read().ok().and_then(|remote_port| {
                        remote_port.parent.upgrade().and_then(|parent_arc| {
                            parent_arc
                                .read()
                                .ok()
                                .and_then(|parent_node| parent_node.description.clone())
                        })
                    })
                })
                .unwrap_or_default();

            ports.push(Port {
                number: port_guard.number as i32,
                remote_node_description: remote_desc,
            });
        }

        ports
    }
}

fn convert_fabric_nodes(
    fabric_nodes: &[Arc<RwLock<discovery::Node>>],
    include_hcas: bool,
) -> Vec<Node> {
    let mut nodes = Vec::new();

    for node_arc in fabric_nodes {
        let node_guard = match node_arc.read() {
            Ok(guard) => guard,
            Err(e) => {
                eprintln!("Failed to read discovered node: {e}");
                continue;
            }
        };

        let description = node_guard
            .description
            .clone()
            .unwrap_or_else(|| "".to_string());

        match node_guard.node_type {
            enums::IbNodeType::Switch => {
                let ports = IbmadDiscoveryService::convert_ports(node_arc);
                nodes.push(Node {
                    guid: node_guard.node_guid,
                    node_description: description,
                    ports,
                    lid: node_guard.lid,
                });
            }
            enums::IbNodeType::CA => {
                if include_hcas {
                    nodes.push(Node {
                        guid: node_guard.node_guid,
                        node_description: description,
                        ports: Vec::new(),
                        lid: node_guard.lid,
                    });
                }
            }
            _ => {}
        }
    }

    nodes
}

impl DiscoverService for IbmadDiscoveryService {
    fn get_nodes(&self) -> Vec<Node> {
        let hca = match ca::get_ca(&self.config.hca) {
            Ok(hca) => hca,
            Err(e) => {
                eprintln!("Failed to load HCA {}: {e}", self.config.hca);
                return Vec::new();
            }
        };

        let mut smp_port = match mad::open_smp_port(&hca) {
            Ok(port) => port,
            Err(e) => {
                eprintln!("Failed to open SMP port for {}: {e}", self.config.hca);
                return Vec::new();
            }
        };

        if let Err(e) = mad::register_agent(&mut smp_port, mad::IB_MGMT_CLASS_DIRECT_ROUTED_SMP) {
            eprintln!(
                "Failed to register direct route agent on SMP port for {}: {e}",
                self.config.hca
            );
            return Vec::new();
        }

        let mut fabric = discovery::Fabric {
            port: smp_port,
            node_map: HashMap::new(),
            nodes: Vec::new(),
            switches: Vec::new(),
            hcas: Vec::new(),
            dr_paths: HashMap::new(),
            ni_timings: Vec::new(),
            retries: self.config.retries,
            timeout: self.config.timeout,
            mad_errors: 0,
            mad_timeouts: 0,
            mads_sent: 0,
            tid: 1,
        };

        if let Err(e) = fabric.seq_discover() {
            eprintln!("Error discovering fabric via ibmad: {e}");
        }

        convert_fabric_nodes(&fabric.nodes, self.config.include_hcas)
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
                    CounterEvent::Exit => {
                        return Ok(());
                    }
                    CounterEvent::Request(nodes) => {
                        let counters = self.get_counters(nodes);
                        if let Err(e) = self.ctr_ev_tx.send(CounterEvent::Response(counters)) {
                            eprintln!("Failed to send counters response: {e}");
                        }
                    }
                    other => {
                        eprintln!("Received unexpected CounterEvent: {other:?}");
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

impl CountersService for IbmadCountersService {
    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>> {
        let hca = match ca::get_ca(&self.config.hca) {
            Ok(hca) => Arc::new(hca),
            Err(e) => {
                eprintln!("Failed to load HCA {}: {e}", self.config.hca);
                return HashMap::new();
            }
        };

        let pool = match ThreadPoolBuilder::new()
            .num_threads(self.config.threads)
            .build()
        {
            Ok(pool) => pool,
            Err(e) => {
                eprintln!("Failed to create thread pool: {e}");
                return HashMap::new();
            }
        };

        let timeout = self.config.timeout;
        let retries = self.config.retries;

        pool.install(|| {
            let num_threads = rayon::current_num_threads().max(1);
            let chunk_size = (lid_ports.len() / num_threads).max(1);

            lid_ports
                .par_chunks(chunk_size)
                .map(|chunk| {
                    let mut local_map: HashMap<(u16, i32), HashMap<String, u64>> = HashMap::new();

                    let ca_ref = Arc::clone(&hca);
                    let mut port = match mad::open_port(ca_ref.as_ref()) {
                        Ok(port) => port,
                        Err(e) => {
                            eprintln!("Failed to open MAD port: {e}");
                            return local_map;
                        }
                    };

                    let agent_id =
                        match mad::register_agent(&mut port, mad::IB_MGMT_CLASS_PERFORMANCE) {
                            Ok(id) => id,
                            Err(e) => {
                                eprintln!("Failed to register performance agent: {e}");
                                return local_map;
                            }
                        };

                    for lp in chunk.iter() {
                        let port_number = match u8::try_from(lp.number) {
                            Ok(num) => num,
                            Err(_) => {
                                eprintln!(
                                    "Port number {} out of range for LID {}",
                                    lp.number, lp.lid
                                );
                                continue;
                            }
                        };

                        let start = Utc::now();
                        let perf = match mad::query_port_counters_extended(
                            &mut port,
                            agent_id,
                            timeout,
                            retries,
                            lp.lid,
                            port_number,
                        ) {
                            Ok(perf) => perf,
                            Err(e) => {
                                eprintln!(
                                    "Failed to query counters for LID {} port {}: {e}",
                                    lp.lid, lp.number
                                );
                                continue;
                            }
                        };
                        let end = Utc::now();

                        let mut counters = perf_mad_to_map(&perf);
                        counters.insert(
                            "start_timestamp".to_string(),
                            start.timestamp_nanos_opt().unwrap_or(0) as u64,
                        );
                        counters.insert(
                            "end_timestamp".to_string(),
                            end.timestamp_nanos_opt().unwrap_or(0) as u64,
                        );

                        local_map.insert((lp.lid, lp.number), counters);
                    }

                    local_map
                })
                .reduce(HashMap::new, |mut acc, mut chunk_map| {
                    acc.extend(chunk_map.drain());
                    acc
                })
        })
    }
}

fn perf_mad_to_map(perf: &mad::perf::perf_mad) -> HashMap<String, u64> {
    let mut counters = HashMap::new();
    counters.insert("xmt_bytes".to_string(), perf.port_xmit_data());
    counters.insert("rcv_bytes".to_string(), perf.port_rcv_data());
    counters.insert("xmt_pkts".to_string(), perf.port_xmit_pkts());
    counters.insert("rcv_pkts".to_string(), perf.port_rcv_pkts());
    counters.insert("xmt_upkts".to_string(), perf.port_unicast_xmit_pkts());
    counters.insert("rcv_upkts".to_string(), perf.port_unicast_rcv_pkts());
    counters.insert("xmt_mpkts".to_string(), perf.port_multicast_xmit_pkts());
    counters.insert("rcv_mpkts".to_string(), perf.port_multicast_rcv_pkts());
    counters.insert("symbol_errors".to_string(), perf.symbol_error_counter());
    counters.insert(
        "link_recovers".to_string(),
        perf.link_error_recovery_counter(),
    );
    counters.insert("link_downed".to_string(), perf.link_downed_counter());
    counters.insert("rcv_errors".to_string(), perf.port_rcv_errors());
    counters.insert(
        "phys_rcv_errors".to_string(),
        perf.port_rcv_remote_physical_errors(),
    );
    counters.insert(
        "switch_rel_errors".to_string(),
        perf.port_rcv_switch_relay_errors(),
    );
    counters.insert("xmt_discards".to_string(), perf.port_xmit_discards());
    counters.insert(
        "xmt_constraint_errors".to_string(),
        perf.port_xmit_constraint_errors(),
    );
    counters.insert(
        "rcv_constraint_errors".to_string(),
        perf.port_rcv_constraint_errors(),
    );
    counters.insert(
        "local_integrity_errors".to_string(),
        perf.local_link_integrity_errors(),
    );
    counters.insert(
        "excess_overrun_errors".to_string(),
        perf.excessive_buffer_overrun_errors(),
    );
    counters.insert("vl15dropped".to_string(), perf.vl15_dropped());
    counters.insert("xmit_waits".to_string(), perf.port_xmit_wait());
    counters.insert("qp1_drops".to_string(), perf.qp1_dropped());

    counters
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibmad::enums::{IbNodeType, IbPortLinkLayerState, IbPortPhyState};

    fn build_node(
        guid: u64,
        lid: u16,
        description: &str,
        node_type: IbNodeType,
    ) -> Arc<RwLock<discovery::Node>> {
        Arc::new(RwLock::new(discovery::Node {
            lid,
            node_type,
            node_guid: guid,
            description: Some(description.to_string()),
            local_port: 1,
            nports: 0,
            ports: Vec::new(),
        }))
    }

    fn add_port(
        node: &Arc<RwLock<discovery::Node>>,
        number: u8,
        lid: u16,
    ) -> Arc<RwLock<discovery::Port>> {
        let port = Arc::new(RwLock::new(discovery::Port {
            number,
            link_state: IbPortLinkLayerState::Active,
            phys_state: IbPortPhyState::LinkUp,
            lid,
            remote_port: None,
            parent: Arc::downgrade(node),
        }));

        {
            let mut node_ref = node.write().expect("node write lock");
            node_ref.ports.push(port.clone());
            node_ref.nports = node_ref.ports.len() as u8;
            if node_ref.lid == 0 {
                node_ref.lid = lid;
            }
        }

        port
    }

    fn link_ports(left: &Arc<RwLock<discovery::Port>>, right: &Arc<RwLock<discovery::Port>>) {
        left.write().expect("left port write lock").remote_port = Some(Arc::downgrade(right));
        right.write().expect("right port write lock").remote_port = Some(Arc::downgrade(left));
    }

    #[test]
    fn convert_ports_populates_remote_description() {
        let switch_a = build_node(1, 10, "switch-a", IbNodeType::Switch);
        let switch_b = build_node(2, 20, "switch-b", IbNodeType::Switch);

        let port_a = add_port(&switch_a, 1, 10);
        let port_b = add_port(&switch_b, 1, 20);
        link_ports(&port_a, &port_b);

        let ports = IbmadDiscoveryService::convert_ports(&switch_a);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].remote_node_description, "switch-b");
    }

    #[test]
    fn convert_fabric_nodes_returns_all_switches() {
        let switch_a = build_node(1, 10, "switch-a", IbNodeType::Switch);
        let switch_b = build_node(2, 20, "switch-b", IbNodeType::Switch);
        let ca_node = build_node(3, 30, "host-ca", IbNodeType::CA);

        let port_a = add_port(&switch_a, 1, 10);
        let port_b = add_port(&switch_b, 1, 20);
        link_ports(&port_a, &port_b);

        let nodes = vec![switch_a.clone(), switch_b.clone(), ca_node];
        let result = convert_fabric_nodes(&nodes, false);

        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|n| n.node_description.starts_with("switch"))
        );
    }

    #[test]
    fn convert_fabric_nodes_includes_hcas_when_requested() {
        let switch = build_node(1, 10, "switch", IbNodeType::Switch);
        let ca_node = build_node(2, 20, "host-ca", IbNodeType::CA);

        add_port(&switch, 1, 10);

        let nodes = vec![switch, ca_node];
        let result_without_hcas = convert_fabric_nodes(&nodes, false);
        let result_with_hcas = convert_fabric_nodes(&nodes, true);

        assert_eq!(result_without_hcas.len(), 1);
        assert_eq!(result_with_hcas.len(), 2);
        assert!(
            result_with_hcas
                .iter()
                .any(|n| n.node_description == "host-ca")
        );
    }

    #[test]
    fn perf_mad_to_map_extracts_expected_counters() {
        let mut perf = mad::perf::perf_mad {
            pm_key: 0,
            reserved: [0; 32],
            data: [0; 192],
        };

        perf.set_port_xmit_data(123);
        perf.set_port_rcv_data(456);
        perf.set_port_xmit_pkts(789);
        perf.set_vl15_dropped(42);

        let counters = perf_mad_to_map(&perf);

        assert_eq!(counters.get("xmt_bytes"), Some(&123));
        assert_eq!(counters.get("rcv_bytes"), Some(&456));
        assert_eq!(counters.get("xmt_pkts"), Some(&789));
        assert_eq!(counters.get("vl15dropped"), Some(&42));
    }
}
