use std::collections::HashMap;

use chrono::prelude::*;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Widget, Table, Row, Cell
    },
};

use crate::{app::{App, CounterMode}, services};

fn truncate_fit(s: &str, max_width: usize) -> String {
    if s.len() > max_width {
        let mut truncated = s[..(max_width.saturating_sub(1))].to_string();
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
}


// Calc columns widths
fn compute_column_widths(total_width: u16, ratios: &[f64]) -> Vec<usize> {
    let total = total_width as f64;
    let mut widths = Vec::with_capacity(ratios.len());
    for &ratio in ratios {
        // At least 2 to avoid zero-width columns.
        let col_width = (ratio * total).round().max(2.0) as usize;
        widths.push(col_width);
    }
    widths
}



// Compute receive/send bandwidth in GB/s based on a performance counter.
pub fn get_bw(perfcounters: &HashMap<String, u64>, counter: &str, counter_mode: &CounterMode) -> f64 {

    let mut time_delta = *perfcounters.get("end_timestamp").unwrap_or(&1) as f64;

    match counter_mode {
        CounterMode::Delta => {
            time_delta = time_delta / 1e9;
        },
        _ => {
            time_delta = 1.0;
        }
    }

    perfcounters
        .get(counter)
        .map(|&val| {
            (val as f64 * 4.0 / (1e9) * 8.0) / time_delta
            //time_delta
        }).unwrap_or(0.0)
}

// Compute bandwidth loss in GB/s based on a performance counter.
pub fn get_bw_loss(perfcounters: &HashMap<String, u64>, counter: &str, counter_mode: &CounterMode) -> f64 {


    let mut time_delta = *perfcounters.get("end_timestamp").unwrap_or(&1) as f64;

    match counter_mode {
        CounterMode::Delta => {
            time_delta = time_delta / 1e9;
        },
        _ => {
            time_delta = 1.0;
        }
    }

    perfcounters
        .get(counter)
        .map(|&val| { 
            val as f64 * 64.0 / (1e9)  / time_delta
        })
        .unwrap_or(0.0)
}

// Count all error counters and return the sum.
pub fn count_errors(perfcounters: &HashMap<String, u64>) -> u128 {
    services::rsmad_services::ERROR_COUNTERS
        .iter()
        .filter_map(|&err_ctr| perfcounters.get(err_ctr))
        .map(|&val| val as u128)
        .sum()
}

// Get Error Strings
pub fn get_error_strings(perfcounters: &HashMap<String, u64>) -> String {
    let errors: Vec<String> = services::rsmad_services::ERROR_COUNTERS
        .iter()
        .filter_map(|&err_ctr| {
            perfcounters.get_key_value(err_ctr)
        })
        .filter(|e| {
            *e.1 > 0
        })
        .map(|e|{
            e.0.to_string()
        }).collect();

        errors.join(",")
}

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
    }
}

impl App {

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

    // Render the top header section with three columns.
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
                Span::from("mlx5_0"), // Replace with your dynamic data if available
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

        // Right Header
        let header_right_text = vec![
            Line::from(vec![
                Span::from("".green()),
            ]),

        ];

        Paragraph::new(header_right_text).render(header_layout[2], buf);
    }

    /// Render the main node table section that shows
    /// LID, NODE, PORTS, RECV_BW, SEND_BW, BW_LOSS, ERRORS.
    fn render_nodes_table(&self, area: Rect, buf: &mut Buffer) {


        // Gather node information
        let mut node_info: Vec<(u16, String, u16, f64, f64, f64, u128, String)> = self
            .nodes
            .iter()
            .map(|n| {
                let counters = self.display_counters.get(&n.lid);

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
                    n.lid,
                    n.node_description.clone(),
                    n.ports as u16,
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
                0 => a.0.cmp(&b.0),
                1 => a.1.cmp(&b.1),
                2 => a.2.cmp(&b.2),
                3 => a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal),
                4 => a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal),
                5 => a.5.partial_cmp(&b.5).unwrap_or(std::cmp::Ordering::Equal),
                6 => a.6.cmp(&b.6),
                7 => a.7.cmp(&b.7),
                _ => std::cmp::Ordering::Equal,
            };

            if self.sort_ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        let available_width = area.width;

        let column_ratios = [0.05, 0.20, 0.05, 0.14, 0.14, 0.14, 0.14, 0.14];
        let widths = compute_column_widths(available_width, &column_ratios);

        let header_cells = vec![
            Cell::from(format!("LID{}", self.get_sort_indicator(0))),
            Cell::from(format!("NODE{}", self.get_sort_indicator(1))),
            Cell::from(format!("PT{}", self.get_sort_indicator(2))),
            Cell::from(format!("RECV_BW{}", self.get_sort_indicator(3))),
            Cell::from(format!("SEND_BW{}", self.get_sort_indicator(4))),
            Cell::from(format!("BW_LOSS{}", self.get_sort_indicator(5))),
            Cell::from(format!("ERR_CNT{}", self.get_sort_indicator(6))),
            Cell::from(format!("ERR_STR{}", self.get_sort_indicator(7))),
        ];

        let header = Row::new(header_cells).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

        let visible_rows = area.height.saturating_sub(1) as usize;
        let offset = self.table_offset.min(node_info.len().saturating_sub(visible_rows));

        let rows = node_info
            .iter()
            .skip(offset)
            .take(visible_rows)
            .map(|(lid, desc, ports, r_bw, x_bw, waits, errs, err_str)| {
                Row::new(vec![
                    Cell::from(format!("{}", lid)),
                    Cell::from(truncate_fit(desc, widths[1])),
                    Cell::from(format!("{}", ports)),
                    Cell::from(format!("{:.2}", r_bw)),
                    Cell::from(format!("{:.2}", x_bw)),
                    Cell::from(format!("{:.2}", waits)),
                    Cell::from(format!("{}", errs)),
                    Cell::from(truncate_fit(err_str, widths[7])),
                ])
            });

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

    /// Render the footer section with three columns.
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
            Line::from(" U = Auto Update".green()),
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
        ];
        Paragraph::new(right_footer_text)
            .block(right_footer_block)
            .render(footer_layout[2], buf);
    }
    
}
