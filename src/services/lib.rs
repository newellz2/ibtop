use std::{
    collections::HashMap, 
    sync::mpsc::{Receiver, Sender}, 
    time::{Instant}
};

use chrono::Utc;
use crate::app::AppConfig;
use super::rsmad::ERROR_COUNTERS;

pub enum ServiceType{
    RsMAD,
    Test
}

#[derive(Clone, Debug)]
pub enum DiscoveryEvent{
    Request,
    Response(Vec<Node>),
    Error,
    Exit
}

#[derive(Clone, Debug)]
pub enum CounterEvent {
    Request(Vec<LidPort>),
    Response(HashMap<(u16, i32), HashMap<String, u64>>),
    Error,
    Exit
}

#[derive(Clone, Debug)]
pub struct Node {
    pub guid: u64,
    pub node_description: String,
    pub ports: Vec<Port>,
    pub lid: u16,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub number: i32,
    pub remote_node_description: String,
}

#[derive(Clone, Debug)]
pub struct LidPort {
    pub lid: u16,
    pub number: i32,
}

pub trait DiscoverService {
    fn get_nodes(&self) -> Vec<Node>;
}

pub trait CountersService {
    fn get_counters(&self, nodes: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>>;
}

// Test services

//Test Discovery Service
pub struct TestDiscoverService {
    ev_disc_rx: Receiver<DiscoveryEvent>,
    disc_ev_tx: Sender<DiscoveryEvent>,
    ports_per_node: usize,
}

impl TestDiscoverService {
    pub fn new(
        ev_disc_rx: Receiver<DiscoveryEvent>,
        disc_ev_tx: Sender<DiscoveryEvent>, 
        _config: AppConfig
    ) -> Self {

        Self{
            ev_disc_rx,
            disc_ev_tx,
            ports_per_node: 64,
        }
    }
    pub fn run(self)  -> color_eyre::Result<()> {

        loop {
            match self.ev_disc_rx.recv() {
                Ok(ev) => {

                    match ev {
                        DiscoveryEvent::Exit => {
                            return Ok(())
                        }
                        DiscoveryEvent::Request => {
                            let _ = self.disc_ev_tx.send(
                                DiscoveryEvent::Response(self.get_nodes())
                            );
                        },
                        _ => {},
                    }
                }
                Err(_e) => {}
            }
        }
    }
}

impl DiscoverService for TestDiscoverService{
    fn get_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();

        // Create a handful of switches with sequential LIDs.
        for i in 1..=1600 {
            let mut ports: Vec<Port> = Vec::new();
            for port_num in 0..self.ports_per_node {
                ports.push(Port {
                    number: port_num as i32,
                    remote_node_description: "".to_string(),
                });
            }
            
            nodes.push(Node {
                guid: i as u64,
                node_description: format!("switch-{i}"),
                ports,
                lid: 16 + i as u16,
            });
        }

        nodes
    }
}

// Test Counters Service
pub struct TestCountersService {
    ev_ctr_rx: Receiver<CounterEvent>,
    ctr_ev_tx: Sender<CounterEvent>,
    start: Instant,
}

impl TestCountersService {
    pub fn new(
        ev_ctr_rx: Receiver<CounterEvent>,
        ctr_ev_tx: Sender<CounterEvent>, 
        config: AppConfig
    ) -> Self {

        Self {
            ev_ctr_rx,
            ctr_ev_tx,
            start: Instant::now(),
        }
    }
    pub fn run(self)  -> color_eyre::Result<()> {

        loop {
            match self.ev_ctr_rx.recv() {
                Ok(ev) => {
                    match ev {
                        CounterEvent::Exit => {
                            return Ok(())
                        }
                        CounterEvent::Request(lid_ports) => {

                            let _ = self.ctr_ev_tx.send(
                                CounterEvent::Response(
                                    self.get_counters(lid_ports)
                                )
                            );
                        },
                        _ => {},
                    }
                }
                Err(_e) => {},
            }
        }
    }
}

impl CountersService for TestCountersService{

    fn get_counters(&self, lid_ports: Vec<LidPort>) -> HashMap<(u16, i32), HashMap<String, u64>>{

        let mut counters: HashMap<(u16, i32), HashMap<String, u64>> = HashMap::new();

        // Calculate a base value using the elapsed time since service start.
        let elapsed = self.start.elapsed().as_secs();
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;

        let simulated_work_duration_nanos = 1_000_000_000; // 1s

        for lp in &lid_ports {
            let mut node_counters: HashMap<String, u64> = HashMap::new();
            // Simple algorithm to generate steadily increasing counters
            let base = elapsed.saturating_mul(1000).saturating_add((lp.lid as i32 + lp.number) as u64 * 10);

            let xmt_bytes = base.saturating_mul(100_000).saturating_mul((lp.lid as i32 + lp.number) as u64);
            node_counters.insert("xmt_bytes".to_string(), xmt_bytes);

            let rcv_bytes = base.saturating_mul(100_000).saturating_mul((lp.lid as i32 + lp.number) as u64);
            node_counters.insert("rcv_bytes".to_string(), rcv_bytes);

            let xmit_waits = base.saturating_mul(10_000).saturating_mul((lp.lid as i32 + lp.number) as u64);
            node_counters.insert("xmit_waits".to_string(), xmit_waits);

            node_counters.insert(
                "start_timestamp".to_string(),
                now_nanos,
            );
            node_counters.insert(
                "end_timestamp".to_string(),
                now_nanos + simulated_work_duration_nanos,
            );

            // Add ErrorCounters
            let _: Vec<_> = ERROR_COUNTERS
                .iter()
                .map(|&err_ctr| {
                    let err_cnt = base.saturating_mul(lp.lid as u64);
                    node_counters.insert(err_ctr.to_string(), err_cnt);
                }).collect();

            counters.insert((lp.lid, lp.number), node_counters);
        }

        counters
    }

}