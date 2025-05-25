use std::{
    collections::HashMap, sync::mpsc::{Receiver, Sender}, thread, time::{Duration, Instant}
};

use chrono::Utc;

use crate::app::AppConfig;

use super::rsmad_services::ERROR_COUNTERS;

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
    Request(Vec<Node>),
    Response(HashMap<u16, HashMap<String, u64>>),
    Error,
    Exit
}

#[derive(Clone, Debug)]
pub struct Node {
    pub guid: u64,
    pub node_description: String,
    pub ports: u64,
    pub lid: u16,
}

pub trait DiscoverService {
    fn get_nodes(&self) -> Vec<Node>;
}

pub trait CountersService {
    fn get_counters(&self, nodes: Vec<Node>) -> HashMap<u16, HashMap<String, u64>>;
}

// Test services

//Test Discovery Service
pub struct TestDiscoverService {
    ev_disc_rx: Receiver<DiscoveryEvent>,
    disc_ev_tx: Sender<DiscoveryEvent>,
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
                Err(_) => {}
            }
        }
    }
}

impl DiscoverService for TestDiscoverService{
    fn get_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();

        // Create a handful of switches with sequential LIDs.
        for i in 1..=1600 {
            nodes.push(Node {
                guid: i as u64,
                node_description: format!("switch-{i}"),
                ports: 64,
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
        _config: AppConfig
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
                        CounterEvent::Request(nodes) => {

                            let _ = self.ctr_ev_tx.send(
                                CounterEvent::Response(
                                    self.get_counters(nodes)
                                )
                            );
                        },
                        _ => {},
                    }
                }
                Err(_) => todo!(),
            }
        }
    }
}

impl CountersService for TestCountersService{
    fn get_counters(&self, nodes: Vec<Node>) -> HashMap<u16, HashMap<String, u64>>{

        let mut counters: HashMap<u16, HashMap<String, u64>> = HashMap::new();

        // Calculate a base value using the elapsed time since service start.
        let elapsed = self.start.elapsed().as_secs();
        let start = Utc::now();
        thread::sleep(Duration::from_secs(1));
        let end = Utc::now();


        for n in &nodes {
            let mut node_counters: HashMap<String, u64> = HashMap::new();
            // Simple algorithm to generate steadily increasing counters
            let base = elapsed * 1000 + n.lid as u64 * 10;
            node_counters.insert("xmt_bytes".to_string(), base * (1e5 as u64) * (n.lid as u64));
            node_counters.insert("rcv_bytes".to_string(), base * (1e5 as u64) * (n.lid as u64));
            node_counters.insert("xmit_waits".to_string(), base * (1e4 as u64) * (n.lid as u64));
            node_counters.insert(
                "start_timestamp".to_string(),
                start.timestamp_nanos_opt().unwrap_or(0) as u64,
            );
            node_counters.insert(
                "end_timestamp".to_string(),
                end.timestamp_nanos_opt().unwrap_or(0) as u64,
            );

            //Add ErrorCounters
            let _: Vec<_> = ERROR_COUNTERS
                .iter()
                .map(|&err_ctr| {
                    node_counters.insert(err_ctr.to_string(), base * (n.lid as u64))
                }).collect();

            counters.insert(n.lid, node_counters);
        }

        counters
    }
}