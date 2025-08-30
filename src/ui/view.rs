
use chrono::prelude::*;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Widget},
};

use crate::{
    app::{
        App, 
        Popup, 
        DETAILS_POPUP_PERCENT_HEIGHT, 
        DETAILS_POPUP_PERCENT_WIDTH, 
        SEARCH_POPUP_LINES_HEIGHT, 
        SEARCH_POPUP_PERCENT_WIDTH,
        AGG_COUNTERS_PORT
    }
};
use super::helpers::{
    truncate_fit, 
    compute_column_widths, 
    get_bw, 
    get_bw_loss, 
    count_errors, 
    get_error_strings,
    centered_rect_percent,
    centered_rect_percent_w_lines_h
};

// Column ratios for the main table layout
const MAIN_TABLE_COLUMN_RATIOS: [f64; 8] = [0.04, 0.32, 0.04, 0.12, 0.12, 0.12, 0.12, 0.12];

// Column ratios for the details popup table layout
const DETAILS_TABLE_COLUMN_RATIOS: [f64; 8] = [0.0, 0.04, 0.32, 0.12, 0.12, 0.12, 0.12, 0.16];

impl Widget for &App {
    // Renders the user interface widgets.
    //
    // This method delegates to smaller functions for clarity:
    //  - `render_header`
    //  - `render_nodes_table`
    //  - `render_footer`
    fn render(self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::vertical([
            Constraint::Length(3),     // Header
            Constraint::Percentage(100), // Node Table
            Constraint::Length(3),     // Footer
        ])
        .split(area);

        // Render the header
        self.render_header(layout[0], buf);

        // Render the node table
        self.render_nodes_table(layout[1], buf);

        // Render the footer
        self.render_footer(layout[2], buf);

        // Render popup
        match self.active_popup {
            Popup::None => {},
            Popup::Search => {
                self.render_search_popup(area, buf);
            },
            Popup::Details => {
                self.render_details_popup(area, buf);
            },
        }
    }
}

impl App {
    /// Returns the sort indicator symbol for a given column.
    /// 
    /// # Arguments
    /// * `col_idx` - The column index to get the sort indicator for
    /// 
    /// # Returns
    /// A string containing the sort indicator ("▲" for ascending, "▼" for descending, or empty)
    fn get_sort_indicator(&self, col_idx: i32) -> &'static str {
        if self.sort_column == col_idx {
            if self.sort_ascending {
                "▲"
            } else {
                "▼"
            }
        } else {
            ""
        }
    }

    /// Render the top header section with three columns showing application status and metadata.
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let utc: DateTime<Utc> = Utc::now();

        let header_layout = Layout::horizontal([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

        // Left Header
        let header_left_text = vec![
            Line::from("ibtop".green()),
            Line::from(vec![
                Span::from("HCA:    ".green()),
                Span::from(
                    &self.config.hca
                ),
            ]),
            Line::from(vec![
                Span::from("Status: ".green()),
                Span::from(format!("{}", self.status)),
            ]),
        ];

        Paragraph::new(header_left_text).render(header_layout[0], buf);

        let last_update_ts = match self.last_counter_update {
            Some(ts) => ts.to_rfc3339_opts(SecondsFormat::Secs, false),
            None => "".to_string(),
        };

        // Middle Header
        let header_mid_text = vec![
            Line::from(vec![
                Span::from("Timestamp: ".green()),
                Span::from(utc.to_rfc3339_opts(SecondsFormat::Secs, false)),
            ]),
            Line::from(vec![
                Span::from("Counters Update: ".green()),
                Span::from(last_update_ts),
            ]),
            Line::from(vec![
                Span::from("Node Count: ".green()),
                Span::from(format!("{}", self.nodes.len())),
            ]),
        ];

        Paragraph::new(header_mid_text).render(header_layout[1], buf);

        // Right Header: show sort and active filter
        let sort_name = match self.sort_column {
            1 => "LID",
            2 => "NODE",
            3 => "PT",
            4 => "RECV_BW",
            5 => "SEND_BW",
            6 => "BW_LOSS",
            7 => "ERR_CNT",
            8 => "ERR_STR",
            _ => "None",
        };
        let sort_text = if self.sort_column >= 1 {
            format!("{}{}", sort_name, self.get_sort_indicator(self.sort_column))
        } else {
            "None".to_string()
        };
        let header_right_text = vec![
            Line::from(vec![
                Span::from("Sort: ".green()),
                Span::from(sort_text),
            ]),
            Line::from(vec![
                Span::from("Filter: ".green()),
                Span::from(self.search_form.value.clone()),
            ]),
        ];

        Paragraph::new(header_right_text).render(header_layout[2], buf);
    }

    /// Render the main node table section that shows node information and performance metrics.
    /// 
    /// Displays columns for: LID, NODE, PORTS, RECV_BW, SEND_BW, BW_LOSS, ERRORS.
    /// Supports filtering by search term and sorting by any column.
    fn render_nodes_table(&self, area: Rect, buf: &mut Buffer) {

        // Create case-insensitive regex for filtering, defaulting to empty string if invalid
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
                4 => a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal), // Receive BW
                5 => a.5.partial_cmp(&b.5).unwrap_or(std::cmp::Ordering::Equal), // Transmit BW
                6 => a.6.partial_cmp(&b.6).unwrap_or(std::cmp::Ordering::Equal), // Xmit waits
                7 => a.7.cmp(&b.7),           // Error count
                8 => a.8.cmp(&b.8),           // Error strings
                _ => std::cmp::Ordering::Equal,
            };

            if self.sort_ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        let available_width = area.width;
        let widths = compute_column_widths(available_width, &MAIN_TABLE_COLUMN_RATIOS);

        let header_cells = vec![
            Cell::from(format!("LID{}", self.get_sort_indicator(1))),
            Cell::from(format!("NODE{}", self.get_sort_indicator(2))),
            Cell::from(format!("PT{}", self.get_sort_indicator(3))),
            Cell::from(format!("RECV_BW{}", self.get_sort_indicator(4))),
            Cell::from(format!("SEND_BW{}", self.get_sort_indicator(5))),
            Cell::from(format!("BW_LOSS{}", self.get_sort_indicator(6))),
            Cell::from(format!("ERR_CNT{}", self.get_sort_indicator(7))),
            Cell::from(format!("ERR_STR{}", self.get_sort_indicator(8))),
        ];

        let header = Row::new(header_cells).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

        let visible_rows = area.height.saturating_sub(1) as usize;
        self.visible_rows.set(visible_rows);
        // Compute a local selection index clamped to filtered data size
        let selected_idx = self.selected.min(node_info.len().saturating_sub(1));
        let offset = self.table_offset.min(node_info.len().saturating_sub(visible_rows));

        let mut rows = node_info
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
            .map(|(idx, (_guid, lid, desc, ports, r_bw, x_bw, waits, errs, err_str))| {
                let mut row = Row::new(vec![
                    Cell::from(format!("{}", lid)),
                    Cell::from(truncate_fit(desc, widths[1])),
                    Cell::from(format!("{}", ports)),
                    Cell::from(format!("{:.2}", r_bw)),
                    Cell::from(format!("{:.2}", x_bw)),
                    Cell::from(format!("{:.2}", waits)),
                    Cell::from(format!("{}", errs)),
                    Cell::from(truncate_fit(err_str, widths[7])),
                ]);
                // Zebra striping for readability (non-selected rows)
                if selected_idx != idx && idx % 2 == 1 {
                    row = row.style(Style::default().bg(Color::Rgb(32, 32, 32)));
                }
                // Highlight the selected row
                if selected_idx == idx {
                    row = row.style(Style::default().bg(Color::LightBlue));
                }
                row
            })
            .collect::<Vec<_>>();

        // If no rows match, show a friendly message row
        if rows.is_empty() {
            rows.push(Row::new(vec![
                Cell::from(""),
                Cell::from("No matching nodes"),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ]));
        }
        

        let constraints = [
            Constraint::Length(widths[0] as u16),
            Constraint::Length(widths[1] as u16),
            Constraint::Length(widths[2] as u16),
            Constraint::Length(widths[3] as u16),
            Constraint::Length(widths[4] as u16),
            Constraint::Length(widths[5] as u16),
            Constraint::Length(widths[6] as u16),
            Constraint::Length(widths[7] as u16),
        ];

        Table::new(rows, constraints)
            .header(header)
            .render(area, buf);

    }

    /// Render the footer section with keyboard shortcuts and application status.
    /// 
    /// Shows three columns with different categories of keyboard shortcuts
    /// and current application state information.
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let footer_layout = Layout::horizontal([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

        // Left Footer
        let left_footer_block = Block::new().border_type(BorderType::Plain).borders(Borders::TOP);
        let left_footer_text = vec![
            Line::from(" d = Fabric Discovery".green()),
            Line::from(" u = Update Counters".green()),
        ];

        Paragraph::new(left_footer_text)
            .block(left_footer_block)
            .render(footer_layout[0], buf);

        // Middle Footer
        let mid_footer_block = Block::new().border_type(BorderType::Plain).borders(Borders::TOP);
        let mid_footer_text = vec![
            Line::from(
                if self.auto_update {
                    " U = Auto Update".yellow()
                } else {
                    " U = Auto Update".green()
                }
            ),
            Line::from(vec![
                Span::from(
                    format!(" W/D/B = Whole/Delta/Baseline: ").green()
                ),
                Span::from(
                    format!("{:?}", self.counter_mode)
                )
            ]),
        ];

        Paragraph::new(mid_footer_text)
            .block(mid_footer_block)
            .render(footer_layout[1], buf);

        // Right Footer
        let right_footer_block = Block::new().border_type(BorderType::Plain).borders(Borders::TOP);
        let right_footer_text = vec![
            Line::from(" s = Sort".green()),
            Line::from(" S = Sort Asc/Desc".green()),
            //Line::from(" PgUp/PgDn/Home/End = Navigate".green()),
        ];

        Paragraph::new(right_footer_text)
            .block(right_footer_block)
            .render(footer_layout[2], buf);

    }

    fn render_search_popup(&self, area: Rect, buf: &mut Buffer) {
        // Don't render search popup if no nodes are available
        if self.nodes.is_empty() {
            return;
        }

        let popup_info = centered_rect_percent_w_lines_h(
            SEARCH_POPUP_PERCENT_WIDTH,
            SEARCH_POPUP_LINES_HEIGHT, 
            area
        );

        let rect = Rect::new(
            popup_info.0, 
            popup_info.1, 
            popup_info.2, 
            popup_info.3,
        );

        Clear.render(rect, buf);
        self.search_form.render(rect, buf);
    }

    fn render_details_popup(&self, area: Rect, buf: &mut Buffer) {
        // Don't render details popup if no node is selected
        if self.selected_node.is_none() {
            return;
        }

        let popup_info = centered_rect_percent(
            DETAILS_POPUP_PERCENT_WIDTH,
            DETAILS_POPUP_PERCENT_HEIGHT, 
            area
        );

        let rect = Rect::new(
            popup_info.0, 
            popup_info.1, 
            popup_info.2, 
            popup_info.3,
        );

        Clear.render(rect, buf);

        let node = self.selected_node.clone().unwrap_or(
            (0, 0, "".to_owned(), 0, 0.0, 0.0, 0.0, 0, "".to_owned())
        );

        let title = format!(
            "Details - Index: {}, GUID: 0x{}, Lid: {}, Desc: {}",
            self.selected,
            node.0,
            node.1,
            node.2
        );

        let block = Block::new()
            .title(title)
            .borders(Borders::ALL);

        let inner_area = block.inner(rect);
        let widths = compute_column_widths(inner_area.width, &DETAILS_TABLE_COLUMN_RATIOS);

        // Prepare node info
        let mut node_info: Vec<(i32, String, f64, f64, f64, u128, String)> = self
            .display_counters
            .iter()
            .map(|(&(lid, port), ctrs)| {
                let node_desc = self
                    .nodes
                    .iter()
                    .find(|n| n.lid == lid)
                    .and_then(|n| n.ports.iter().find(|p| p.number == port))
                    .map(|p| p.remote_node_description.clone())
                    .unwrap_or("".to_string());
                let recv_bw = get_bw(ctrs, "rcv_bytes", &self.counter_mode);
                let xmt_bw = get_bw(ctrs, "xmt_bytes", &self.counter_mode);
                let xmit_waits = get_bw_loss(ctrs, "xmit_waits", &self.counter_mode);
                let error_count = count_errors(ctrs);
                let error_strings = get_error_strings(ctrs);
                (
                    port,
                    node_desc,
                    recv_bw,
                    xmt_bw,
                    xmit_waits,
                    error_count,
                    error_strings,
                )
            })
            .collect();

        node_info.sort_by(|a, b| a.0.cmp(&b.0));

        let visible_rows = inner_area.height.saturating_sub(1) as usize;
        self.visible_rows.set(visible_rows);
        let offset = self.popup_table_offset.min(node_info.len().saturating_sub(visible_rows));

        let mut rows = node_info
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
            .map(|(idx, (port, node_desc, r_bw, x_bw, waits, errs, err_str))| {
                let mut row = Row::new(vec![
                    Cell::from(format!("{}", port)),
                    Cell::from(truncate_fit(node_desc, widths[2])),
                    Cell::from(format!("{:.2}", r_bw)),
                    Cell::from(format!("{:.2}", x_bw)),
                    Cell::from(format!("{:.2}", waits)),
                    Cell::from(format!("{}", errs)),
                    Cell::from(truncate_fit(err_str, widths[7])),
                ]);
                // Zebra striping for readability (non-selected)
                if self.popup_selected != idx && idx % 2 == 1 {
                    row = row.style(Style::default().bg(Color::Rgb(32, 32, 32)));
                }
                // Highlight the selected row in the popup
                if self.popup_selected == idx {
                    row = row.style(Style::default().bg(Color::LightBlue));
                }
                row
            })
            .collect::<Vec<_>>();

        if rows.is_empty() {
            rows.push(Row::new(vec![
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ]));
        }

        let header_cells = vec![
            Cell::from(format!("PT")),
            Cell::from(format!("NODE")),
            Cell::from(format!("RECV_BW")),
            Cell::from(format!("SEND_BW")),
            Cell::from(format!("BW_LOSS")),
            Cell::from(format!("ERR_CNT")),
            Cell::from(format!("ERR_STR")),
        ];

        let header = Row::new(header_cells).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

        let constraints = [
            Constraint::Length(widths[1] as u16),
            Constraint::Length(widths[2] as u16),
            Constraint::Length(widths[3] as u16),
            Constraint::Length(widths[4] as u16),
            Constraint::Length(widths[5] as u16),
            Constraint::Length(widths[6] as u16),
            Constraint::Length(widths[7] as u16),
        ];

        let table = Table::new(rows, constraints)
            .header(header);

        table.render(inner_area, buf);
        
        block.render(rect, buf);
    }
}
