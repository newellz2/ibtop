use std::{cmp::Ordering, collections::HashMap, cell::Cell};

use chrono::{DateTime, Utc};
use config::Config;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers}, layout::{Offset, Rect}, DefaultTerminal, Terminal
};

use crate::{
    event::{AppEvent, Event, EventHandler}, services::lib::{CounterEvent, DiscoveryEvent, Node}, ui::{forms::StringField, helpers::centered_rect}, Args
};

#[derive(Debug)]
pub enum CounterMode {
    Whole,
    Delta,
    Baseline,
}

#[derive(Debug, Default, serde_derive::Deserialize, PartialEq, Clone)]
pub struct AppConfig {
    pub hca: String,
    pub pkey: u32,
    pub threads: usize,
    pub service_type: String,
    pub update_interval: usize,
    pub include_hcas: bool,
    pub timeout: u32,
}

// Main application state.
pub struct App {
    pub running: bool,
    pub config: AppConfig,
    pub nodes: Vec<Node>,

    // Counters
    pub display_counters: HashMap<u16, HashMap<String, u64>>,
    pub current_counters: HashMap<u16, HashMap<String, u64>>,
    pub previous_counters: HashMap<u16, HashMap<String, u64>>,
    pub baseline_counters: HashMap<u16, HashMap<String, u64>>,

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

    pub popup_rect: Rect,

    /// Search field for filtering results
    pub search_field: StringField,

    /// Current scroll offset for the nodes table
    pub table_offset: usize,

    /// Number of visible rows in the table (set during rendering)
    pub visible_rows: Cell<usize>,

    /// Currently selected table row
    pub selected: usize,

    /// Show popup with node details
    pub show_popup: bool,

    /// Manages all event handling (tick, crossterm, discovery, counters).
    pub events: EventHandler,
}

impl App {
    //  constructor
    pub fn new(args: Args) -> Self {
        let config = Config::builder()
        .add_source(
            config::Environment::with_prefix("IBTOP")
                .try_parsing(true)
                .separator("_")
                .list_separator(" "),
        )
        .build()
        .unwrap();

        let app_config: AppConfig = match config.try_deserialize() {
            Ok(c) =>{
                c
            }
            Err(_)=> {
                AppConfig{
                    hca: args.hca,
                    timeout: args.timeout,
                    threads: args.threads,
                    pkey: args.pkey,
                    update_interval: args.update_interval,
                    include_hcas: args.include_hcas,
                    service_type: args.service_type,
                }
            }
        };

        let mut app = App {
            config: app_config.clone(),
            running: true,
            status: "".into(),
            search_field: StringField::new("Search"),
            popup_rect:  Rect::new(0, 0, 0, 0),
            nodes: Vec::new(),

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
            sort_column: -1,
            sort_ascending: false,
            table_offset: 0,
            visible_rows: Cell::new(0),
            selected: 0,
            show_popup: false,
            events: EventHandler::new(app_config),
        };

        app.discover_fabric();
        app
    }

    /// Run the application, drawing the UI and handling events until it is no longer `running`.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        while self.running {

            if self.show_popup {
                let _ = terminal.show_cursor();
            } else {
                let _ = terminal.hide_cursor();
            }
            
            // Render the UI
            terminal.draw(|frame| {
                let area = frame.area();

                if self.show_popup {
                    let centered_rect = centered_rect(60, 3, area);
                    let offset = Offset{
                        x: (centered_rect.0 as i32 + self.search_field.cursor_offset().x).into(),
                        y: (centered_rect.1 + 1).into(),
                    };
                    let cursor_offset = area.offset(offset);
                    frame.set_cursor_position(cursor_offset);
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
                    self.status = "Discovery Complete".into();
                    self.nodes = nodes;
                }
                _ => {
                    self.status = "Unknown Discovery Event".into();
                }
            },
            Event::Counters(counter_event) => match counter_event {
                CounterEvent::Response(counters) => {
                    self.handle_counters_update(counters);
                    self.last_counter_update = Some(Utc::now());
                }
                _ => {
                    self.status = "Unknown Counters Event".into();
                }
            },
        }
        Ok(())
    }

    // Handle keyboard inputs.
    fn handle_key_event(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {

        // Handling for the search popup
        if self.show_popup {
            match key_event {

                KeyEvent { code: KeyCode::Esc, .. }
                | KeyEvent { code: KeyCode::Enter, .. } => {
                    self.show_popup = false;
                }

                // Other key presses go to the search field
                _ => self.search_field.on_key_press(key_event),
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
                    let max_idx = self.nodes.len().saturating_sub(1);
                    self.selected = (self.selected + 1).min(max_idx);

                    let vis = self.visible_rows.get().max(1);
                    let max_offset = self.nodes.len().saturating_sub(vis);
                    if self.selected >= self.table_offset + vis {
                        self.table_offset = (self.table_offset + 1).min(max_offset);
                    }
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
                    if self.selected < self.table_offset {
                        self.table_offset = self.table_offset.saturating_sub(1);
                    }
                }
            }

            // Show popup
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                if !self.nodes.is_empty() {
                    self.show_popup = true;
                    self.popup_rect.width = 10;
                    self.popup_rect.height = 10;
                    self.popup_rect.x = 10;
                    self.popup_rect.y = 10;
                }
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
            self.status = "Counters update is already pending…".into();
            return;
        }
        if self.nodes.is_empty() {
            self.status = "No nodes to update counters for.".into();
            return;
        }

        self.status = "Updating counters…".into();
        self.pending_counter_update = true;
        self.events.send(AppEvent::Counters(CounterEvent::Request(
            self.nodes.clone(),
        )));
    }

    //Populate the counters
    fn handle_counters_update(&mut self, counters: HashMap<u16, HashMap<String, u64>>) {

        self.previous_counters = std::mem::take(&mut self.current_counters);
        self.current_counters = counters;

        match self.counter_mode {
            CounterMode::Whole => {
                // Just replace the entire map
                self.display_counters = self.current_counters.clone();
                self.status = format!("Updated counters ({})", self.display_counters.len());

            }
            CounterMode::Delta => {
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
        self.tick = (self.tick + 1) % 30; // Reset tick after 29
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

    // Increments the sort column, preventing integer overflow.
    fn increment_sort_column(&mut self) {
        self.sort_column = (self.sort_column + 1) % 8
    }

    // Cleanly shuts down the application.
    fn quit(&mut self) {
        self.running = false;
    }
}

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
                new_val
            }
        };
        output.insert(key.clone(), delta);
    }

    output
}