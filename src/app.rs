use std::{cell::Cell, cmp::Ordering, collections::HashMap};

use chrono::{DateTime, Utc};
use config::Config;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers}, layout::Offset, DefaultTerminal
};

use crate::{
    event::{AppEvent, Event, EventHandler}, services::lib::{CounterEvent, DiscoveryEvent, LidPort, Node}, 
    ui::{forms::{NodeDetailsForm, SearchForm}, 
    helpers::{centered_rect_percent_w_lines_h, count_errors, get_bw, get_bw_loss, get_error_strings}}, Args
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

#[derive(Debug, Default, serde::Deserialize, PartialEq, Clone)]
pub struct AppConfig {
    pub hca: String,
    pub pkey: u32,
    pub threads: usize,
    pub service_type: String,
    pub update_interval: usize,
    pub include_hcas: bool,
    pub timeout: u32,
    pub retries: u32,
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

impl App {
    ///  Constructor
    pub fn new(args: Args) -> Self {
        let app_config: AppConfig = Config::builder()
            .add_source(
                config::Environment::with_prefix("IBTOP")
                    .try_parsing(true)
                    .separator("_")
                    .list_separator(" "),
            )
            .build()
            .and_then(|c| c.try_deserialize())
            .unwrap_or_else(|_| AppConfig {
                hca: args.hca,
                timeout: args.timeout,
                retries: args.retries,
                threads: args.threads,
                pkey: args.pkey,
                update_interval: args.update_interval,
                include_hcas: args.include_hcas,
                service_type: args.service_type,
            });

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

        app.discover_fabric();
        app
    }

    /// Run the application, drawing the UI and handling events until it is no longer `running`.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        while self.running {

            match self.active_popup{
                Popup::None => {
                    let _ = terminal.hide_cursor();
                },
                Popup::Details => {
                    let _ = terminal.hide_cursor();
                },
                Popup::Search => {
                    let _ = terminal.show_cursor();
                },
            }

            // Render the UI
            terminal.draw(|frame| {

                let area = frame.area();

                match self.active_popup {

                    Popup::Search => { // Seatrch Popup
                        let centered_rect = centered_rect_percent_w_lines_h(
                            SEARCH_POPUP_PERCENT_WIDTH, 
                            SEARCH_POPUP_LINES_HEIGHT, 
                            area
                        );

                        let offset = Offset{
                            x: (centered_rect.0 as i32 + self.search_form.cursor_offset().x).into(),
                            y: (centered_rect.1 + 1).into(),
                        };

                        let cursor_offset = area.offset(offset);
                        frame.set_cursor_position(cursor_offset);
                    },
                    _ => {},
                }

                frame.render_widget(&self, area);
            })?;

            // Process the next event
            self.handle_events()?;
        }
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
                Popup::None => {},
                Popup::Search => {
                    match key_event {

                        KeyEvent { code: KeyCode::Esc, .. }
                        | KeyEvent { code: KeyCode::Enter, .. } => {
                            self.active_popup = Popup::None;
                            if !self.nodes.is_empty() {
                                self.selected = 0;
                                self.set_selected_node_guid();
                            }
                        }

                        // Other key presses go to the search field
                        _ => self.search_form.on_key_press(key_event),
                    }
                },
                Popup::Details => {
                    match key_event {
                        KeyEvent { code: KeyCode::Esc, .. }
                        | KeyEvent { code: KeyCode::Enter, .. } => {
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
                                    self.popup_table_offset = (self.popup_table_offset + 1).min(max_offset);
                                }
                                
                            }
                        }

                    // Move selection up
                    KeyEvent {
                        code: KeyCode::Up,
                        ..
                    } => {
                        if !self.display_counters.is_empty() {
                            if self.popup_selected > 0 {
                                self.popup_selected -= 1;
                            }

                            if self.popup_selected < self.popup_table_offset {
                                self.popup_table_offset = self.popup_table_offset.saturating_sub(1);
                            }                            
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
                        _ => {}
                    }
                },
            }
            return Ok(());
        }

        match key_event {
            // Quit keys: ESC, 'q', or Ctrl-C
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE, kind: _, state: _
            }
            | KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE, kind: _, state: _
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL, kind: _, state: _} => {
                self.events.send(AppEvent::Quit);
            }

            // Discovery request
            KeyEvent {
                code: KeyCode::Char('d'),
                ..
            } => {
                self.discover_fabric();
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
                code: KeyCode::Up,
                ..
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
            KeyEvent { code: KeyCode::PageDown, .. } => {
                if !self.nodes.is_empty() {
                    let vis = self.visible_rows.get().max(1);
                    let len = self.filtered_len();
                    self.selected = (self.selected + vis).min(len.saturating_sub(1));
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Page up
            KeyEvent { code: KeyCode::PageUp, .. } => {
                if !self.nodes.is_empty() {
                    let vis = self.visible_rows.get().max(1);
                    self.selected = self.selected.saturating_sub(vis);
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // Home (go to first row)
            KeyEvent { code: KeyCode::Home, .. } => {
                if !self.nodes.is_empty() {
                    self.selected = 0;
                    self.set_selected_node_guid();
                    self.ensure_selected_visible();
                }
            }

            // End (go to last row)
            KeyEvent { code: KeyCode::End, .. } => {
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
                    self.display_counters.clear();
                    self.current_counters.clear();
                    self.previous_counters.clear();
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
        self.events.send(AppEvent::Discover(DiscoveryEvent::Request));
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

        self.status = "Updating counters...".into();
        self.pending_counter_update = true;

        let lid_ports: Vec<LidPort> = match self.active_popup {

            Popup::Details => {

                match &self.selected_node {
                    Some(node) => {
                        
                        let node_option = self.nodes.iter().find(|n| n.guid == node.0);

                        match node_option {
                            Some(node) => {
                                self.status = node.node_description.clone();

                                let lid = node.lid;
                                node.ports.iter().map(|p|{
                                    LidPort{
                                        lid,
                                        number: p.number
                                    }
                                }).collect()
                            },
                            None => {
                                self.nodes.iter().map(|n| {
                                            LidPort {
                                                lid : n.lid,
                                                number: AGG_COUNTERS_PORT,
                                            }
                                        }).collect()
                            },
                        }
                    },
                    None => {
                        self.nodes.iter().map(|n| {
                                    LidPort {
                                        lid : n.lid,
                                        number: AGG_COUNTERS_PORT,
                                    }
                                }).collect()
                    }
                }

            }
            // Everything else
            _ => {
                self.nodes.iter().map(|n| {
                            LidPort {
                                lid : n.lid,
                                number: AGG_COUNTERS_PORT,
                            }
                        }).collect()
            }

        };

        self.events.send(AppEvent::Counters(CounterEvent::Request(
            lid_ports
        )));
    }

    /// Populate the counters
    fn handle_counters_update(&mut self, counters: HashMap<(u16, i32), HashMap<String, u64>>) {

        self.previous_counters = std::mem::take(&mut self.current_counters);
        self.current_counters = counters;

        match self.counter_mode {
            CounterMode::Whole => {
                // Just replace the entire map
                self.display_counters = self.current_counters.clone();
                self.status = format!("Updated counters ({})", self.display_counters.len());

            }
            CounterMode::Delta => {
                self.display_counters.clear();
                // For each LID in the incoming counters, mutate the old counters in place
                for (lid, new_map) in &self.current_counters {
                    if let Some(old_map) = self.previous_counters.get_mut(&lid) {
                        let delta = calc_counters_delta(old_map, &new_map);
                        self.display_counters.insert(*lid,  delta);
                    } else {
                        // If we had no previous entry for that LID, just insert the new one
                        self.display_counters.insert(*lid, new_map.clone());
                    }
                }

                self.status = format!("Updated counters ({})",  self.current_counters.len());

            }
            CounterMode::Baseline => {
                self.display_counters.clear();
                for (lid, new_map) in & self.current_counters {
                    if let Some(old_map) = self.baseline_counters.get_mut(&lid) {
                        let delta = calc_counters_delta(&old_map, &new_map);
                        self.display_counters.insert(*lid,  delta);
                    } else {
                        // If we had no previous entry for that LID, just insert the new one
                        self.display_counters.insert(*lid, new_map.clone());
                    }
                }
                self.status = format!("Updated counters ({})", self.current_counters.len());
            }
        }

        self.pending_counter_update = false;
    }

    // Called every tick (roughly 30fps by default).
    fn on_tick(&mut self) {
        self.tick = (self.tick + 1) % TICK_RESET_INTERVAL; // Reset tick after TICK_RESET_INTERVAL - 1
        if self.tick == 0 {
            self.auto_update_counter += 1;
        }

        if self.auto_update &&
           self.pending_counter_update == false &&
           self.auto_update_counter >= self.auto_update_interval {
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
        self
            .nodes
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

                let recv_bw = counters
                    .map_or(0.0, |ctrs| get_bw(ctrs, "rcv_bytes", &self.counter_mode));
                let xmt_bw = counters
                    .map_or(0.0, |ctrs| get_bw(ctrs, "xmt_bytes", &self.counter_mode));
                let xmit_waits = counters
                    .map_or(0.0, |ctrs| get_bw_loss(ctrs, "xmit_waits", &self.counter_mode));
                let error_count = counters
                    .map_or(0, |ctrs| count_errors(ctrs));
                let error_strings = counters
                    .map_or("".to_string(), |ctrs| get_error_strings(ctrs));
                
                (
                    n.guid,
                    n.lid,
                    n.node_description.clone(),
                    n.ports.len() as u16,
                    recv_bw,
                    xmt_bw,
                    xmit_waits,
                    error_count,
                    error_strings
                )
            })
            .collect();

        // Sort based on `self.sort_column`
        node_info.sort_by(|a, b| {
            let ordering = match self.sort_column {
                1 => a.1.cmp(&b.1),           // LID
                2 => a.2.cmp(&b.2),           // Description
                3 => a.3.cmp(&b.3),           // Port count
                4 => a.4.partial_cmp(&b.4).unwrap_or(Ordering::Equal), // Receive BW
                5 => a.5.partial_cmp(&b.5).unwrap_or(Ordering::Equal), // Transmit BW
                6 => a.6.partial_cmp(&b.6).unwrap_or(Ordering::Equal), // Xmit waits
                7 => a.7.cmp(&b.7),           // Error count
                8 => a.8.cmp(&b.8),           // Error strings
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
            Ordering::Equal | Ordering::Greater => {
                new_val.saturating_sub(old_val)
            }
            _ => {
                // Counter likely reset, use new value as-is
                new_val
            }
        };
        output.insert(key.clone(), delta);
    }

    output
}