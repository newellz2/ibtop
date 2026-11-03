use std::{
    cell::Cell,
    cmp::Ordering,
    collections::HashMap,
};

use chrono::{DateTime, Utc};
use ratatui::{
    DefaultTerminal,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Offset,
};

use crate::{
    Args,
    event::{AppEvent, Event, EventHandler},
    scope::read_scope_file,
    services::lib::{CounterEvent, DiscoveryEvent, LidPort, Node},
    ui::{
        forms::{NodeDetailsForm, SearchForm},
        helpers::{
            centered_rect_percent_w_lines_h, count_errors, get_bw, get_bw_loss, get_error_strings,
        },
    },
};

pub const SEARCH_POPUP_PERCENT_WIDTH: u16 = 60;
pub const SEARCH_POPUP_LINES_HEIGHT: u16 = 3;

pub const DETAILS_POPUP_PERCENT_WIDTH: u16 = 90;
pub const DETAILS_POPUP_PERCENT_HEIGHT: u16 = 80;

pub const AGG_COUNTERS_PORT: i32 = 255;
pub const TICK_RESET_INTERVAL: usize = 30;
pub const MAX_SORT_COLUMNS: i32 = 9;

/// Represents different modes for displaying counter data.
#[derive(Debug)]
pub enum CounterMode {
    /// Display raw counter values
    Whole,
    /// Display delta values (difference from previous update)
    Delta,
    /// Display values relative to a baseline
    Baseline,
}

/// Represents the currently active popup dialog.
#[derive(Debug, PartialEq)]
pub enum Popup {
    /// No popup is active
    None,
    /// Search popup is active
    Search,
    /// Node details popup is active
    Details,
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct AppConfig {
    pub hca: String,
    pub pkey_index: u16,
    pub threads: usize,
    pub service_type: String,
    pub update_interval: usize,
    pub include_hcas: bool,
    pub sl: u8,
    pub timeout: u32,
    pub retries: u32,
    pub scope_file: Option<String>,
}

impl From<Args> for AppConfig {
    /// Clap already merged flags, `IBTOP_*` env vars and defaults.
    fn from(args: Args) -> Self {
        AppConfig {
            hca: args.hca,
            pkey_index: args.pkey_index,
            threads: args.threads,
            service_type: args.service_type,
            update_interval: args.update_interval,
            include_hcas: args.include_hcas,
            sl: args.sl,
            timeout: args.timeout,
            retries: args.retries,
            scope_file: args.scope_file,
        }
    }
}

// Main application state.
pub struct App {
    pub running: bool,
    pub config: AppConfig,
    pub nodes: Vec<Node>,

    /// Selected Node
    pub selected_node: Option<(u64, u16, String, u16, f64, f64, f64, u128, String)>,

    /// Counters
    pub display_counters: HashMap<(u16, i32), HashMap<String, u64>>,
    pub current_counters: HashMap<(u16, i32), HashMap<String, u64>>,
    pub previous_counters: HashMap<(u16, i32), HashMap<String, u64>>,
    pub baseline_counters: HashMap<(u16, i32), HashMap<String, u64>>,

    pub pending_counter_update: bool,
    /// Nodes in the last request that were skipped because they have no LID
    /// (e.g. a switch whose port-0 PortInfo timed out). They stay in the
    /// table, but LID-routed PerfMgt queries can't reach them.
    pub no_lid_nodes: usize,
    pub update_start_time: Option<DateTime<Utc>>,
    pub last_counter_update: Option<DateTime<Utc>>,
    pub counter_mode: CounterMode,

    pub status: String,
    pub tick: usize,
    pub auto_update: bool,
    pub auto_update_interval: usize,
    pub auto_update_counter: usize,

    pub sort_column: i32,
    pub sort_ascending: bool,

    /// Search field for filtering results
    pub search_form: SearchForm,

    /// NodeDetails form
    pub node_details_form: NodeDetailsForm,

    /// Current scroll offset for the nodes table
    pub table_offset: usize,

    /// Popup table offset
    pub popup_table_offset: usize,

    /// Track the selected port
    pub popup_selected: usize,

    /// Number of visible rows in the table (set during rendering)
    pub visible_rows: Cell<usize>,

    /// Currently selected table row
    pub selected: usize,

    /// Active popup
    pub active_popup: Popup,

    /// Manages all event handling (tick, crossterm, discovery, counters).
    pub events: EventHandler,
}

pub type MainNodeInfo = (u64, u16, String, u16, f64, f64, f64, u128, String);

impl App {
    ///  Constructor
    pub fn new(args: Args) -> Self {
        let app_config = AppConfig::from(args);

        let mut app = App {
            config: app_config.clone(),
            running: true,
            status: "".into(),
            search_form: SearchForm::new("Search"),
            node_details_form: NodeDetailsForm::new("Details"),
            nodes: Vec::new(),
            selected_node: None,
            display_counters: HashMap::new(),
            current_counters: HashMap::new(),
            previous_counters: HashMap::new(),
            baseline_counters: HashMap::new(),
            pending_counter_update: false,
            no_lid_nodes: 0,
            update_start_time: None,
            counter_mode: CounterMode::Whole,
            last_counter_update: None,

            tick: 0,
            auto_update: false,
            auto_update_interval: app_config.update_interval,
            auto_update_counter: 0,
            sort_column: 0,
            sort_ascending: false,
            table_offset: 0,
            popup_table_offset: 0,
            popup_selected: 0,
            visible_rows: Cell::new(0),
            selected: 0,
            active_popup: Popup::None,
            events: EventHandler::new(app_config),
        };
        if app.config.scope_file.is_some() {
            let scope_file = app.config.scope_file.as_ref().unwrap();
            app.status = format!("Using scope file");
            let nodes = read_scope_file(&scope_file);
            app.nodes.extend(nodes);
        } else {
            app.discover_fabric();
        }
        app
    }

    /// Run the application, drawing the UI and handling events until it is no longer `running`.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        // Initial draw so the UI appears immediately.
        self.draw(&mut terminal)?;

        while self.running {
            // Block waiting for the next event (tick/input/service), then redraw.
            self.handle_events()?;
            if !self.running {
                break;
            }
            self.draw(&mut terminal)?;
        }
        Ok(())
    }

    fn draw(&self, terminal: &mut DefaultTerminal) -> color_eyre::Result<()> {
        match self.active_popup {
            Popup::None | Popup::Details => {
                let _ = terminal.hide_cursor();
            }
            Popup::Search => {
                let _ = terminal.show_cursor();
            }
        }

        terminal.draw(|frame| {
            let area = frame.area();

            if self.active_popup == Popup::Search {
                let centered_rect = centered_rect_percent_w_lines_h(
                    SEARCH_POPUP_PERCENT_WIDTH,
                    SEARCH_POPUP_LINES_HEIGHT,
                    area,
                );

                let offset = Offset {
                    x: (centered_rect.0 as i32 + self.search_form.cursor_offset().x).into(),
                    y: (centered_rect.1 + 1).into(),
                };

                let cursor_offset = area.offset(offset);
                frame.set_cursor_position(cursor_offset);
            }

            frame.render_widget(self, area);
        })?;

        Ok(())
    }

    /// Handle inbound events from the [`EventHandler`] channels.
    fn handle_events(&mut self) -> color_eyre::Result<()> {
        match self.events.next()? {
            Event::Tick => self.on_tick(),
            Event::Crossterm(event) => {
                if let crossterm::event::Event::Key(key_event) = event {
                    self.handle_key_event(key_event)?;
                }
            }
            Event::App(app_event) => {
                if let AppEvent::Quit = app_event {
                    self.quit();
                }
            }
            Event::Discover(discovery_event) => match discovery_event {
                DiscoveryEvent::Response(nodes) => {
                    self.status = format!("Discovery complete: {} nodes found", nodes.len());
                    self.nodes = nodes;
                    if !self.nodes.is_empty() {
                        self.selected = 0;
                        self.set_selected_node_guid();
                    }
                }
                DiscoveryEvent::Error => {
                    self.status = "Discovery failed".into();
                }
                DiscoveryEvent::Exit => {
                    // Discovery service is shutting down
                }
                _ => {
                    self.status = "Unknown discovery event".into();
                }
            },
            Event::Counters(counter_event) => match counter_event {
                CounterEvent::Response(counters) => {
                    self.handle_counters_update(counters);
                    self.last_counter_update = Some(Utc::now());
                }
                CounterEvent::Error => {
                    self.status = "Counter update failed".into();
                    self.pending_counter_update = false;
                }
                CounterEvent::Exit => {
                    // Counter service is shutting down
                }
                _ => {
                    self.status = "Unknown counter event".into();
                }
            },
        }
        Ok(())
    }

    /// Handle keyboard inputs.
    fn handle_key_event(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        // Handling for popup
        if self.active_popup != Popup::None {
            match self.active_popup {
                Popup::None => {}
                Popup::Search => {
                    match key_event {
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        }
                        | KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            self.active_popup = Popup::None;
                            if !self.nodes.is_empty() {
                                self.selected = 0;
                                self.set_selected_node_guid();
                            }
                        }

                        // Other key presses go to the search field
                        _ => self.search_form.on_key_press(key_event),
                    }
                }
                Popup::Details => {
                    match key_event {
                        KeyEvent {
                            code: KeyCode::Esc, ..
                        }
                        | KeyEvent {
                            code: KeyCode::Enter,
                            ..
                        } => {
                            self.active_popup = Popup::None;
                        }

                        // Move selection down
                        KeyEvent {
                            code: KeyCode::Down,
                            ..
                        } => {
                            if !self.display_counters.is_empty() {
                                let max_idx = self.display_counters.len().saturating_sub(1);
                                self.popup_selected = (self.popup_selected + 1).min(max_idx);

                                let vis = self.visible_rows.get().max(1);
                                let max_offset = self.display_counters.len().saturating_sub(vis);

                                if self.popup_selected >= self.popup_table_offset + vis {
                                    self.popup_table_offset =
                                        (self.popup_table_offset + 1).min(max_offset);
                                }
                            }
                        }

                        // Move selection up
                        KeyEvent {
                            code: KeyCode::Up, ..
                        } => {
                            if !self.display_counters.is_empty() {
                                if self.popup_selected > 0 {
                                    self.popup_selected -= 1;
                                }

                                if self.popup_selected < self.popup_table_offset {
                                    self.popup_table_offset =
                                        self.popup_table_offset.saturating_sub(1);
                                }
                            }
                        }

                        // Update counters
                        KeyEvent {
                            code: KeyCode::Char('u'),
                            ..
                        } => {
                            if self.nodes.is_empty() {
                                self.status =
                                    "No nodes discovered yet, cannot update counters.".into();
                            } else {
                                self.update_counters();
                            }
                        }
                        _ => {}
                    }
                }
            }
            return Ok(());
        }

        match key_event {
            // Quit keys: ESC, 'q', or Ctrl-C
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                kind: _,
                state: _,
            }
            | KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                kind: _,
                state: _,
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                kind: _,
                state: _,
            } => {
                self.events.send(AppEvent::Quit);
            }

            // Discovery request
            KeyEvent {
                code: KeyCode::Char('d'),
                ..
            } => {
                if self.config.scope_file.is_some() {
                    self.status = "Scope file mode, skipping discovery.".into();
                    let scope_file = self.config.scope_file.as_ref().unwrap();
                    let nodes = read_scope_file(&scope_file);
                    self.nodes = nodes;
                    if !self.nodes.is_empty() {
                        self.selected = 0;
                        self.set_selected_node_guid();
                    }
                } else {
                    self.discover_fabric();
                }
            }

            // Update counters
            KeyEvent {
                code: KeyCode::Char('u'),
                ..
            } => {
                if self.nodes.is_empty() {
                    self.status = "No nodes discovered yet, cannot update counters.".into();
                } else {
                    self.update_counters();
                }
            }

            // Enable auto-update
            KeyEvent {
                code: KeyCode::Char('U'),
                ..
            } => {
                self.auto_update = !self.auto_update;
            }

            // Whole Counters
            KeyEvent {
                code: KeyCode::Char('W'),
                ..
            } => {
                self.counter_mode = CounterMode::Whole;
            }

            // Delta Counters
            KeyEvent {
                code: KeyCode::Char('D'),
                ..
            } => {
                self.counter_mode = CounterMode::Delta;
            }

            // Baseline Counters
            KeyEvent {
                code: KeyCode::Char('B'),
                ..
            } => {
                self.baseline_counters = self.current_counters.clone();
                self.counter_mode = CounterMode::Baseline;
            }

            // Cycle sort column
            KeyEvent {
                code: KeyCode::Char('s'),
                ..
            } => {
                self.increment_sort_column();
            }

            KeyEvent {
                code: KeyCode::Char('S'),
                ..
            } => {
                self.sort_ascending = !self.sort_ascending;
            }

            // Move selection down
            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                if !self.nodes.is_empty() {
                    let max_idx = self.filtered_len().saturating_sub(1);
                    self.selected = (self.selected + 1).min(max_idx);
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Move selection up
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                if !self.nodes.is_empty() {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Page down
            KeyEvent {
                code: KeyCode::PageDown,
                ..
            } => {
                if !self.nodes.is_empty() {
                    let vis = self.visible_rows.get().max(1);
                    let len = self.filtered_len();
                    self.selected = (self.selected + vis).min(len.saturating_sub(1));
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Page up
            KeyEvent {
                code: KeyCode::PageUp,
                ..
            } => {
                if !self.nodes.is_empty() {
                    let vis = self.visible_rows.get().max(1);
                    self.selected = self.selected.saturating_sub(vis);
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Home (go to first row)
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                if !self.nodes.is_empty() {
                    self.selected = 0;
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // End (go to last row)
            KeyEvent {
                code: KeyCode::End, ..
            } => {
                if !self.nodes.is_empty() {
                    let len = self.filtered_len();
                    self.selected = len.saturating_sub(1);
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Show popup
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.set_selected_node_guid();

                if self.selected_node.is_some() {
                    // Only the view is reset. The per-port sample store is
                    // kept so the popup's first Delta refresh already has a
                    // previous sample to diff against.
                    self.display_counters.clear();
                    self.popup_table_offset = 0;
                    self.popup_selected = 0;
                    self.active_popup = Popup::Details;
                } else {
                    self.active_popup = Popup::None;
                }
            }

            // Show Search popup
            KeyEvent {
                code: KeyCode::Char('/'),
                ..
            } => {
                self.active_popup = Popup::Search;
            }

            _ => {}
        }
        Ok(())
    }

    // Discover Fabric
    fn discover_fabric(&mut self) {
        self.status = "Discovering...".into();
        self.events
            .send(AppEvent::Discover(DiscoveryEvent::Request));
    }

    // Update Counters
    fn update_counters(&mut self) {
        if self.pending_counter_update {
            self.status = "Counters update is already pending...".into();
            return;
        }
        if self.nodes.is_empty() {
            self.status = "No nodes discovered yet, cannot update counters.".into();
            return;
        }

        let selected = match (&self.active_popup, &self.selected_node) {
            (Popup::Details, Some(sel)) => self.nodes.iter().find(|n| n.guid == sel.0),
            _ => None,
        };

        // LID-0 nodes stay in the table but get no PerfMgt queries: there's
        // no LID to route them to. `per_port_targets` already drops ports
        // without a LID (which covers every port of a LID-0 switch).
        let (lid_ports, no_lid_nodes): (Vec<LidPort>, usize) = match selected {
            Some(node) => {
                let targets = per_port_targets(node);
                if targets.is_empty() {
                    self.status = format!(
                        "{}: no LID assigned, not polling counters",
                        node.node_description
                    );
                    return;
                }
                self.status = node.node_description.clone();
                (targets, 0)
            }
            None => {
                let mut no_lid = 0;
                let mut targets = Vec::new();
                for n in &self.nodes {
                    let node_targets = if n.is_ca {
                        // CA PMAs don't do AllPortSelect; query each port and
                        // aggregate in handle_counters_update.
                        per_port_targets(n)
                    } else if n.lid != 0 {
                        vec![LidPort {
                            lid: n.lid,
                            number: AGG_COUNTERS_PORT,
                        }]
                    } else {
                        Vec::new()
                    };
                    if node_targets.is_empty() {
                        no_lid += 1;
                    }
                    targets.extend(node_targets);
                }
                self.status = "Updating counters...".into();
                (targets, no_lid)
            }
        };

        self.no_lid_nodes = no_lid_nodes;
        self.pending_counter_update = true;
        self.update_start_time = Some(Utc::now());

        self.events
            .send(AppEvent::Counters(CounterEvent::Request(lid_ports)));
    }

    /// Populate the counters
    ///
    /// `current_counters` is a running store of the latest raw sample for
    /// every (lid, port) ever answered; `previous_counters` is that store as
    /// it was before this response. Deltas are taken per (lid, port) against
    /// the last sample *of that same key*, so a target that missed a refresh
    /// (or was last polled from the other view) diffs against its own last
    /// sample over the real elapsed time, instead of being shown as a
    /// lifetime total. Keys with no earlier sample are left blank for one
    /// refresh. CA totals are summed from the per-port results afterwards,
    /// so a GPU port dropping out can't look like a counter reset.
    fn handle_counters_update(&mut self, counters: HashMap<(u16, i32), HashMap<String, u64>>) {
        self.previous_counters = self.current_counters.clone();
        for (key, sample) in &counters {
            self.current_counters.insert(*key, sample.clone());
        }

        let mut display = match self.counter_mode {
            CounterMode::Whole => counters.clone(),
            CounterMode::Delta => deltas_against(&self.previous_counters, &counters),
            CounterMode::Baseline => deltas_against(&self.baseline_counters, &counters),
        };
        aggregate_ca_counters(&self.nodes, &mut display);

        let mut status = format!("Updated counters ({})", counters.len());
        if !counters.is_empty() && display.is_empty() {
            status.push_str(", collecting first sample for deltas");
        }
        if self.no_lid_nodes > 0 {
            status.push_str(&format!(
                ", {} node(s) without LID not polled",
                self.no_lid_nodes
            ));
        }
        self.status = status;
        self.display_counters = display;
        self.pending_counter_update = false;
    }

    // Called every tick.
    fn on_tick(&mut self) {
        self.tick = (self.tick + 1) % TICK_RESET_INTERVAL; // Reset tick after TICK_RESET_INTERVAL - 1
        if self.tick == 0 {
            self.auto_update_counter += 1;
        }

        if self.auto_update
            && self.pending_counter_update == false
            && self.auto_update_counter >= self.auto_update_interval
        {
            if !self.nodes.is_empty() {
                self.status = "Updating counters...".into();
                self.update_counters();
            }
            // Reset
            self.auto_update_counter = 0;
        }
    }

    /// Increments the sort column, cycling through available columns (0-8).
    /// Column 0 means no sorting, columns 1-8 correspond to different data fields.
    fn increment_sort_column(&mut self) {
        self.sort_column = (self.sort_column + 1) % MAX_SORT_COLUMNS;
    }

    // Cleanly shuts down the application.
    fn quit(&mut self) {
        self.running = false;
    }

    /// Number of rows after applying the current filter
    fn filtered_len(&self) -> usize {
        let re = regex::RegexBuilder::new(&self.search_form.value)
            .case_insensitive(true)
            .build()
            .unwrap_or_else(|_| regex::Regex::new("").unwrap());
        self.nodes
            .iter()
            .filter(|n| re.is_match(&n.node_description))
            .count()
    }

    /// Keep `table_offset` in sync so the selected row stays visible.
    fn ensure_selected_visible(&mut self) {
        let vis = self.visible_rows.get().max(1);
        let len = self.filtered_len();
        let max_offset = len.saturating_sub(vis);

        if self.selected < self.table_offset {
            self.table_offset = self.selected.min(max_offset);
        } else if self.selected >= self.table_offset + vis {
            let new_offset = (self.selected + 1).saturating_sub(vis);
            self.table_offset = new_offset.min(max_offset);
        } else {
            self.table_offset = self.table_offset.min(max_offset);
        }
    }

    fn set_selected_node_guid(&mut self) {
        // Create regex for filtering, defaulting to empty string if invalid
        let re = regex::RegexBuilder::new(&self.search_form.value)
            .case_insensitive(true)
            .build()
            .unwrap_or_else(|_| regex::Regex::new("").unwrap());

        // Filter and gather node information
        let mut node_info: Vec<(u64, u16, String, u16, f64, f64, f64, u128, String)> = self
            .nodes
            .iter()
            .filter(|n| re.is_match(&n.node_description))
            .map(|n| {
                let counters = self.display_counters.get(&(n.lid, AGG_COUNTERS_PORT));

                let recv_bw =
                    counters.map_or(0.0, |ctrs| get_bw(ctrs, "rcv_bytes", &self.counter_mode));
                let xmt_bw =
                    counters.map_or(0.0, |ctrs| get_bw(ctrs, "xmt_bytes", &self.counter_mode));
                let xmit_waits = counters.map_or(0.0, |ctrs| {
                    get_bw_loss(ctrs, "xmit_waits", &self.counter_mode)
                });
                let error_count = counters.map_or(0, |ctrs| count_errors(ctrs));
                let error_strings = counters.map_or("".to_string(), |ctrs| get_error_strings(ctrs));

                (
                    n.guid,
                    n.lid,
                    n.node_description.clone(),
                    n.ports.len() as u16,
                    recv_bw,
                    xmt_bw,
                    xmit_waits,
                    error_count,
                    error_strings,
                )
            })
            .collect();

        // Sort based on `self.sort_column`
        node_info.sort_by(|a, b| {
            let ordering = match self.sort_column {
                1 => a.1.cmp(&b.1),                                    // LID
                2 => a.2.cmp(&b.2),                                    // Description
                3 => a.3.cmp(&b.3),                                    // Port count
                4 => a.4.partial_cmp(&b.4).unwrap_or(Ordering::Equal), // Receive BW
                5 => a.5.partial_cmp(&b.5).unwrap_or(Ordering::Equal), // Transmit BW
                6 => a.6.partial_cmp(&b.6).unwrap_or(Ordering::Equal), // Xmit waits
                7 => a.7.cmp(&b.7),                                    // Error count
                8 => a.8.cmp(&b.8),                                    // Error strings
                _ => Ordering::Equal,
            };

            if self.sort_ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        // Clamp selection to available rows and set the selected GUID
        if self.selected >= node_info.len() {
            self.selected = node_info.len().saturating_sub(1);
        }

        if let Some(selected_node) = node_info.get(self.selected) {
            self.selected_node = Some(selected_node.clone());
        } else {
            // Clear selection if no valid node found
            self.selected_node = None;
        }
    }
}

/// One counter query per port of `node`, each addressed to that port's LID.
/// Ports without an assigned LID (e.g. still in Init) can't be queried.
fn per_port_targets(node: &Node) -> Vec<LidPort> {
    node.ports
        .iter()
        .filter(|p| p.lid != 0)
        .map(|p| LidPort {
            lid: p.lid,
            number: p.number,
        })
        .collect()
}

/// CA nodes are queried per port; sum their port counters into the
/// `(node.lid, AGG_COUNTERS_PORT)` entry the main table reads. Timestamps are
/// combined as earliest start / latest end so rate math still works.
fn aggregate_ca_counters(
    nodes: &[Node],
    counters: &mut HashMap<(u16, i32), HashMap<String, u64>>,
) {
    // A LID-0 CA has no unique key to aggregate under; several of them would
    // collide on (0, AGG_COUNTERS_PORT).
    for n in nodes.iter().filter(|n| n.is_ca && n.lid != 0) {
        let mut agg: HashMap<String, u64> = HashMap::new();
        for p in &n.ports {
            let Some(port_ctrs) = counters.get(&(p.lid, p.number)) else {
                continue;
            };
            for (k, &v) in port_ctrs {
                let entry = agg.entry(k.clone());
                match k.as_str() {
                    "start_timestamp" => {
                        entry.and_modify(|e| *e = (*e).min(v)).or_insert(v);
                    }
                    "end_timestamp" => {
                        entry.and_modify(|e| *e = (*e).max(v)).or_insert(v);
                    }
                    _ => {
                        entry.and_modify(|e| *e = e.saturating_add(v)).or_insert(v);
                    }
                }
            }
        }
        if !agg.is_empty() {
            counters.insert((n.lid, AGG_COUNTERS_PORT), agg);
        }
    }
}

/// Per-key deltas of `new` against `base`, omitting keys `base` has never seen.
fn deltas_against(
    base: &HashMap<(u16, i32), HashMap<String, u64>>,
    new: &HashMap<(u16, i32), HashMap<String, u64>>,
) -> HashMap<(u16, i32), HashMap<String, u64>> {
    new.iter()
        .filter_map(|(key, new_map)| {
            base.get(key)
                .map(|old_map| (*key, calc_counters_delta(old_map, new_map)))
        })
        .collect()
}

/// Calculate the delta between two counter maps.
///
/// This function computes the difference between new and old counter values.
/// If the new value is less than the old value (indicating a counter reset),
/// it returns the new value as-is.
fn calc_counters_delta(
    old_map: &HashMap<String, u64>,
    new_map: &HashMap<String, u64>,
) -> HashMap<String, u64> {
    let mut output = HashMap::new();

    for (key, &new_val) in new_map {
        let old_val = old_map.get(key).copied().unwrap_or(0);

        let delta = match new_val.cmp(&old_val) {
            Ordering::Equal | Ordering::Greater => new_val.saturating_sub(old_val),
            _ => {
                // Counter likely reset, use new value as-is
                new_val
            }
        };
        output.insert(key.clone(), delta);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::lib::Port;

    fn port(number: i32, lid: u16) -> Port {
        Port {
            number,
            lid,
            remote_node_description: String::new(),
            link_state: "Active".into(),
        }
    }

    fn ctrs(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn gpu() -> Node {
        Node {
            guid: 1,
            node_description: "gpu".into(),
            // Port 3 is Init with no LID yet.
            ports: vec![port(1, 100), port(2, 101), port(3, 0)],
            lid: 100,
            is_ca: true,
        }
    }

    #[test]
    fn per_port_targets_uses_port_lids_and_skips_unassigned() {
        let t = per_port_targets(&gpu());
        let got: Vec<(u16, i32)> = t.iter().map(|lp| (lp.lid, lp.number)).collect();
        assert_eq!(got, vec![(100, 1), (101, 2)]);
    }

    #[test]
    fn aggregate_sums_ca_ports_and_merges_timestamps() {
        let nodes = vec![gpu()];
        let mut c = HashMap::new();
        c.insert(
            (100, 1),
            ctrs(&[("xmt_bytes", 10), ("link_downed", 1), ("start_timestamp", 50), ("end_timestamp", 60)]),
        );
        c.insert(
            (101, 2),
            ctrs(&[("xmt_bytes", 5), ("link_downed", 2), ("start_timestamp", 40), ("end_timestamp", 70)]),
        );

        aggregate_ca_counters(&nodes, &mut c);

        let agg = &c[&(100, AGG_COUNTERS_PORT)];
        assert_eq!(agg["xmt_bytes"], 15);
        assert_eq!(agg["link_downed"], 3);
        assert_eq!(agg["start_timestamp"], 40);
        assert_eq!(agg["end_timestamp"], 70);
        // Per-port entries are left intact for the Details view.
        assert_eq!(c[&(101, 2)]["xmt_bytes"], 5);
    }

    #[test]
    fn deltas_skip_keys_without_a_previous_sample() {
        let mut base = HashMap::new();
        base.insert((7, 1), ctrs(&[("link_downed", 5), ("end_timestamp", 1_000)]));
        let mut new = HashMap::new();
        new.insert((7, 1), ctrs(&[("link_downed", 6), ("end_timestamp", 3_000)]));
        // Never sampled before: must not show its lifetime total as a delta.
        new.insert((8, 1), ctrs(&[("link_downed", 500), ("end_timestamp", 3_000)]));

        let d = deltas_against(&base, &new);

        assert_eq!(d.len(), 1);
        assert_eq!(d[&(7, 1)]["link_downed"], 1);
        assert_eq!(d[&(7, 1)]["end_timestamp"], 2_000);
    }

    #[test]
    fn ca_total_delta_unaffected_by_a_port_missing_a_refresh() {
        // Delta per port first, then sum: a GPU port that drops out for one
        // refresh just contributes nothing, instead of the summed total
        // shrinking and being mistaken for a counter reset.
        let nodes = vec![gpu()];
        let mut prev = HashMap::new();
        prev.insert((100, 1), ctrs(&[("xmt_bytes", 1_000), ("end_timestamp", 10)]));
        prev.insert((101, 2), ctrs(&[("xmt_bytes", 9_000), ("end_timestamp", 10)]));
        let mut new = HashMap::new();
        new.insert((100, 1), ctrs(&[("xmt_bytes", 1_100), ("end_timestamp", 20)]));
        // (101, 2) didn't answer this time.

        let mut display = deltas_against(&prev, &new);
        aggregate_ca_counters(&nodes, &mut display);

        assert_eq!(display[&(100, AGG_COUNTERS_PORT)]["xmt_bytes"], 100);
        assert_eq!(display[&(100, AGG_COUNTERS_PORT)]["end_timestamp"], 10);
    }

    #[test]
    fn aggregate_skips_ca_with_no_responses_and_ignores_switches() {
        let mut sw = gpu();
        sw.is_ca = false;
        sw.lid = 7;
        let nodes = vec![gpu(), sw];
        let mut c = HashMap::new();
        c.insert((7, AGG_COUNTERS_PORT), ctrs(&[("xmt_bytes", 99)]));

        aggregate_ca_counters(&nodes, &mut c);

        assert!(!c.contains_key(&(100, AGG_COUNTERS_PORT)));
        assert_eq!(c[&(7, AGG_COUNTERS_PORT)]["xmt_bytes"], 99);
    }
}
