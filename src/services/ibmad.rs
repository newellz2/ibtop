use super::lib::{CounterEvent, CountersService, DiscoverService, DiscoveryEvent, Node};
use crate::{
    app::AppConfig,
    services::lib::{LidPort, Port},
};
use chrono::Utc;
use ibmad::mad;
use rayon::prelude::*;
use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender},
};
use tracing::{error, warn};

pub const ERROR_COUNTERS: [&str; 9] = [
    "symbol_errors",
    "link_recovers",
    "link_downed",
    "rcv_errors",
    "phys_rcv_errors",
    "switch_rel_errors",
    "excess_overrun_errors",
    "vl15dropped",
    "qp1_drops",
];

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
                        // Terminate thread
                        return Ok(());
                    }
                    DiscoveryEvent::Request => {
                        let nodes = self.get_nodes();
                        // Send the response even if empty
                        if let Err(e) = self.disc_ev_tx.send(DiscoveryEvent::Response(nodes)) {
                            error!("Failed to send discovery response: {e}");
                        }
                    }
                    // Log unknown events for debugging
                    _ => {
                        warn!("Received unexpected DiscoveryEvent: {ev:?}");
                    }
                },
                // If the sender is gone, we can exit or continue
                Err(e) => {
                    error!("DiscoveryService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl DiscoverService for IbmadDiscoveryService {
    fn get_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();

        // Get the HCA
        let hca = match ibmad::ca::get_ca(&self.config.hca) {
            Ok(ca) => ca,
            Err(e) => {
                error!("Failed to get HCA '{}': {e}", self.config.hca);
                return nodes;
            }
        };

        // Open an SMP port for discovery
        let port = match ibmad::mad::open_smp_port(&hca) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to open SMP port: {e}");
                return nodes;
            }
        };

        // Register DR SMP agent
        let mut port = port;
        let agent_id = match ibmad::mad::register_agent(
            &mut port,
            ibmad::mad::IB_MGMT_CLASS_DIRECT_ROUTED_SMP,
        ) {
            Ok(id) => id,
            Err(e) => {
                error!("Failed to register DR SMP agent: {e}");
                return nodes;
            }
        };

        // Create fabric and discover
        let mut fabric = ibmad::discovery::Fabric {
            port,
            agent_id,
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

        // This project targets NVLink-style fabrics; use the NVLink-specific traversal.
        if let Err(e) = fabric.seq_discover_nvlink() {
            error!("Error discovering fabric: {e}");
            return nodes;
        }

        // Build port connections map
        let mut port_connections: HashMap<(u64, u8), String> = HashMap::new();

        for node_arc in &fabric.nodes {
            let node_ref = match node_arc.read() {
                Ok(r) => r,
                Err(_) => continue,
            };

            for port_arc in &node_ref.ports {
                let port_ref = match port_arc.read() {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                if let Some(weak_remote) = &port_ref.remote_port {
                    if let Some(remote_port_arc) = weak_remote.upgrade() {
                        if let Ok(remote_port_ref) = remote_port_arc.read() {
                            if let Some(remote_node_arc) = remote_port_ref.parent.upgrade() {
                                if let Ok(remote_node_ref) = remote_node_arc.read() {
                                    port_connections.insert(
                                        (node_ref.node_guid, port_ref.number),
                                        remote_node_ref.description.clone().unwrap_or_default(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Convert fabric nodes to our Node type
        for node_arc in &fabric.nodes {
            let node_ref = match node_arc.read() {
                Ok(r) => r,
                Err(_) => continue,
            };

            match node_ref.node_type {
                ibmad::enums::IbNodeType::CA => {
                    if self.config.include_hcas {
                        nodes.push(Node {
                            guid: node_ref.node_guid,
                            node_description: node_ref.description.clone().unwrap_or_default(),
                            ports: Vec::new(),
                            lid: node_ref.lid,
                        });
                    }
                }
                ibmad::enums::IbNodeType::Switch => {
                    let ports: Vec<Port> = node_ref
                        .ports
                        .iter()
                        .filter_map(|port_arc| {
                            let port_ref = port_arc.read().ok()?;

                            // Skip port 0 (management port) and inactive ports
                            // Only include ports that are Active or Init state
                            if port_ref.number == 0 {
                                return None;
                            }
                            if port_ref.link_state != ibmad::enums::IbPortLinkLayerState::Active
                                && port_ref.link_state != ibmad::enums::IbPortLinkLayerState::Init
                            {
                                return None;
                            }

                            let remote_desc = port_connections
                                .get(&(node_ref.node_guid, port_ref.number))
                                .cloned()
                                .unwrap_or_default();

                            Some(Port {
                                number: port_ref.number as i32,
                                remote_node_description: remote_desc,
                            })
                        })
                        .collect();

                    nodes.push(Node {
                        guid: node_ref.node_guid,
                        node_description: node_ref.description.clone().unwrap_or_default(),
                        ports,
                        lid: node_ref.lid,
                    });
                }
                _ => {}
            }
        }

        nodes
    }
}

// Counters service
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
        tracing::info!("IbmadCountersService started");
        loop {
            match self.ev_ctr_rx.recv() {
                Ok(ev) => match ev {
                    CounterEvent::Exit => {
                        tracing::info!("IbmadCountersService exiting");
                        return Ok(());
                    }
                    CounterEvent::Request(nodes) => {
                        tracing::debug!(
                            "IbmadCountersService: Request received for {} nodes",
                            nodes.len()
                        );
                        let counters = self.get_counters(nodes);
                        tracing::debug!(
                            "IbmadCountersService: Sending response with {} entries",
                            counters.len()
                        );
                        if let Err(e) = self.ctr_ev_tx.send(CounterEvent::Response(counters)) {
                            error!("Failed to send counters response: {e}");
                        }
                    }
                    _ => {
                        warn!("Received unexpected CounterEvent: {ev:?}");
                    }
                },
                Err(e) => {
                    error!("CountersService channel closed: {e}");
                    return Ok(());
                }
            }
        }
    }
}

impl CountersService for IbmadCountersService {
    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>> {
        let hca_name = self.config.hca.clone();
        let timeout = self.config.timeout;
        let retries = self.config.retries;

        // Get HCA (to create ports in threads)
        let hca = match ibmad::ca::get_ca(&hca_name) {
            Ok(ca) => ca,
            Err(e) => {
                error!("Failed to get HCA '{}': {e}", hca_name);
                return HashMap::new();
            }
        };

        lid_ports
            .into_par_iter()
            .map_init(
                || {
                    // Init: open port and register agent per thread
                    match ibmad::mad::open_port(&hca) {
                        Ok(mut p) => {
                            match ibmad::mad::register_agent(
                                &mut p,
                                ibmad::mad::IB_MGMT_CLASS_PERFORMANCE,
                            ) {
                                Ok(id) => Some((p, id)),
                                Err(e) => {
                                    error!("Failed to register perf agent in thread: {e}");
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to open MAD port in thread: {e}");
                            None
                        }
                    }
                },
                |state, lp| {
                    let (port, agent_id) = state.as_mut()?;

                    let start = Utc::now();
                    let res = mad::query_port_counters_extended(
                        port,
                        *agent_id,
                        timeout,
                        retries,
                        lp.lid,
                        lp.number as u8,
                    );
                    let end = Utc::now();

                    let perf_mad = match res {
                        Ok(mad) => mad,
                        Err(e) => {
                            // Log the error but continue
                            tracing::debug!(
                                "Failed to query counters for LID {} Port {}: {e}",
                                lp.lid,
                                lp.number
                            );
                            return None;
                        }
                    };

                    let mut perfctrs: HashMap<String, u64> = HashMap::new();

                    // Counters
                    perfctrs.insert("xmt_bytes".to_string(), perf_mad.port_xmit_data());
                    perfctrs.insert("rcv_bytes".to_string(), perf_mad.port_rcv_data());
                    perfctrs.insert("xmit_waits".to_string(), perf_mad.port_xmit_wait());

                    // Errors
                    perfctrs.insert("symbol_errors".to_string(), perf_mad.symbol_error_counter());
                    perfctrs.insert(
                        "link_recovers".to_string(),
                        perf_mad.link_error_recovery_counter(),
                    );
                    perfctrs.insert("link_downed".to_string(), perf_mad.link_downed_counter());
                    perfctrs.insert("rcv_errors".to_string(), perf_mad.port_rcv_errors());
                    perfctrs.insert(
                        "phys_rcv_errors".to_string(),
                        perf_mad.port_rcv_remote_physical_errors(),
                    );
                    perfctrs.insert(
                        "switch_rel_errors".to_string(),
                        perf_mad.port_rcv_switch_relay_errors(),
                    );
                    perfctrs.insert(
                        "excess_overrun_errors".to_string(),
                        perf_mad.excessive_buffer_overrun_errors(),
                    );
                    perfctrs.insert("vl15dropped".to_string(), perf_mad.vl15_dropped());
                    perfctrs.insert("qp1_drops".to_string(), perf_mad.qp1_dropped());

                    // Additional
                    perfctrs.insert("xmit_discards".to_string(), perf_mad.port_xmit_discards());
                    perfctrs.insert("xmit_pkts".to_string(), perf_mad.port_xmit_pkts());
                    perfctrs.insert("rcv_pkts".to_string(), perf_mad.port_rcv_pkts());

                    // Timestamps
                    perfctrs.insert(
                        "start_timestamp".to_string(),
                        start.timestamp_nanos_opt().unwrap_or(0) as u64,
                    );
                    perfctrs.insert(
                        "end_timestamp".to_string(),
                        end.timestamp_nanos_opt().unwrap_or(0) as u64,
                    );

                    Some(((lp.lid, lp.number), perfctrs))
                },
            )
            .filter_map(|x| x)
            .collect()
    }
}
