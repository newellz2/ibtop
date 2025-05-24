use color_eyre::eyre::WrapErr;
use ratatui::crossterm::event::{self, Event as CrosstermEvent};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{app::AppConfig, services::{
    lib::{CounterEvent, DiscoveryEvent, TestCountersService, TestDiscoverService},
    rsmad_services::{RsmadCountersService, RsmadDiscoveryService},
}};

/// The frequency (in Hz) at which tick events are emitted.
const TICK_FPS: f64 = 30.0;

#[derive(Clone, Debug)]
pub enum Event {
    Tick,
    Crossterm(CrosstermEvent),
    App(AppEvent),
    Discover(DiscoveryEvent),
    Counters(CounterEvent),
}

// Application events.
//
#[derive(Clone, Debug)]
pub enum AppEvent {
    Discover(DiscoveryEvent),
    Counters(CounterEvent),
    Quit,
}

// Terminal event handler responsible for spawning and managing
// background threads and channels for various event types.
pub struct EventHandler {
    _config: AppConfig,

    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,

    disc_tx: mpsc::Sender<DiscoveryEvent>,
    disc_rx: mpsc::Receiver<DiscoveryEvent>,

    ctr_tx: mpsc::Sender<CounterEvent>,
    ctr_rx: mpsc::Receiver<CounterEvent>,

    wait_duration: Duration,
}

impl EventHandler {
    // Constructs a new [`EventHandler`] and spawns new threads for:
    //  1) Generating tick & crossterm events
    //  2) Discovery service
    //  3) Counters service
    //
    // These threads communicate with the main event loop via channels.
    pub fn new(config: AppConfig) -> Self {

        // 1) Spawn the general event thread (tick + crossterm).
        let (sender, receiver) = mpsc::channel();
        let sender_clone = sender.clone();
        thread::spawn(move || {
            let actor = EventThread::new(sender_clone);
            if let Err(e) = actor.run() {
                eprintln!("Error in EventThread: {e}");
            }
        });

        // 2) Spawn the discovery service thread.
        let (disc_tx, ev_disc_rx) = mpsc::channel::<DiscoveryEvent>();
        let (disc_ev_tx, disc_rx) = mpsc::channel::<DiscoveryEvent>();
        {
            let config_clone = config.clone();
            let service_type_clone = config.service_type.clone();
            thread::spawn(move || {
                match service_type_clone.as_str() {
                    "test" => {
                        let disc_actor =
                            TestDiscoverService::new(ev_disc_rx, disc_ev_tx, config_clone);
                        let _ = disc_actor.run();
                    }
                    // Default
                    _ => {
                        let disc_actor =
                            RsmadDiscoveryService::new(ev_disc_rx, disc_ev_tx, config_clone);
                        let _ = disc_actor.run();
                    }
                }
            });
        }

        // 3) Spawn the counters service thread.
        let (ctr_tx, ev_ctx_rx) = mpsc::channel::<CounterEvent>();
        let (ctr_ev_tx, ctr_rx) = mpsc::channel::<CounterEvent>();
        {
            let config_clone = config.clone();
            let service_type_clone = config.service_type.clone();
            thread::spawn(move || {
                match service_type_clone.as_str() {
                    "test" => {
                        let ctr_actor =
                            TestCountersService::new(ev_ctx_rx, ctr_ev_tx, config_clone);
                        let _ = ctr_actor.run();
                    }
                    // Default
                    _ => {
                        let ctr_actor =
                            RsmadCountersService::new(ev_ctx_rx, ctr_ev_tx, config_clone);
                        let _ = ctr_actor.run();
                    }
                }
            });
        }

        Self {
            _config: config,
            sender,
            receiver,
            disc_tx,
            disc_rx,
            ctr_tx,
            ctr_rx,
            wait_duration: Duration::from_millis(1),
        }
    }

    // Blocks until an event is received from any of the three channels:
    //  - General event receiver (tick, crossterm, app)
    //  - Discovery event receiver
    //  - Counter event receiver
    pub fn next(&self) -> color_eyre::Result<Event> {
        loop {
            // 1) General events
            if let Ok(e) = self.receiver.recv_timeout(self.wait_duration) {
                return Ok(e);
            }
            // 2) Discovery events
            if let Ok(e) = self.disc_rx.recv_timeout(self.wait_duration) {
                return Ok(Event::Discover(e));
            }
            // 3) Counter events
            if let Ok(e) = self.ctr_rx.recv_timeout(self.wait_duration) {
                return Ok(Event::Counters(e));
            }
        }
    }

    pub fn send(&mut self, app_event: AppEvent) {
        match app_event {
            AppEvent::Discover(DiscoveryEvent::Request) => {
                let _ = self.disc_tx.send(DiscoveryEvent::Request);
            }
            AppEvent::Counters(CounterEvent::Request(nodes)) => {
                let _ = self.ctr_tx.send(CounterEvent::Request(nodes));
            }
            _ => {
                let _ = self.sender.send(Event::App(app_event));
            }
        }
    }
}

// A thread that handles reading crossterm events and emitting tick events on a regular schedule.
struct EventThread {
    sender: mpsc::Sender<Event>,
}

impl EventThread {
    /// Constructs a new instance of [`EventThread`].
    fn new(sender: mpsc::Sender<Event>) -> Self {
        Self { sender }
    }

    fn run(self) -> color_eyre::Result<()> {
        let tick_interval = Duration::from_secs_f64(1.0 / TICK_FPS);
        let mut last_tick = Instant::now();

        loop {
            // Emit a tick event if our interval has passed.
            let elapsed = last_tick.elapsed();
            if elapsed >= tick_interval {
                last_tick = Instant::now();
                self.send(Event::Tick);
            } else {
                // We'll wait the remaining time for a crossterm event
                // if there is any leftover time in this tick.
                let remaining = tick_interval.saturating_sub(elapsed);
                if event::poll(remaining).wrap_err("failed to poll for crossterm events")? {
                    let ev = event::read().wrap_err("failed to read crossterm event")?;
                    self.send(Event::Crossterm(ev));
                }
            }
        }
    }

    /// Sends an event to the receiver, ignoring any send errors (e.g., if the receiver is dropped).
    fn send(&self, event: Event) {
        let _ = self.sender.send(event);
    }
}
