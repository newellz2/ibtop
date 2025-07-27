
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
        SEARCH_POPUP_LINES_HEIGHT, SEARCH_POPUP_PERCENT_WIDTH}
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

        let re = regex::Regex::new(&self.search_form.value).unwrap_or_else(|_| {
            regex::Regex::new("").unwrap()
        });

        // Filter and gather node information
        let mut node_info: Vec<(u64, u16, String, u16, f64, f64, f64, u128, String)> = self
            .nodes
            .iter()
            .filter(|n| {  
                re.is_match(&n.node_description)
            })
            .map(|n| {
                let counters = self.display_counters.get(&(n.lid, 255));

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
                1 => a.1.cmp(&b.1),
                2 => a.2.cmp(&b.2),
                3 => a.3.cmp(&b.3),
                4 => a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal),
                5 => a.5.partial_cmp(&b.5).unwrap_or(std::cmp::Ordering::Equal),
                6 => a.6.partial_cmp(&b.6).unwrap_or(std::cmp::Ordering::Equal),
                7 => a.7.cmp(&b.7),
                8 => a.8.cmp(&b.8),
                _ => std::cmp::Ordering::Equal,
            };

            if self.sort_ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        let available_width = area.width;

        let column_ratios = [0.04, 0.32, 0.04, 0.12, 0.12, 0.12, 0.12, 0.12];
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
        self.visible_rows.set(visible_rows);
        let offset = self.table_offset.min(node_info.len().saturating_sub(visible_rows));

        let rows = node_info
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
            .map(|(idx, (
                    _guid, lid, desc, ports, r_bw, x_bw, waits, errs, err_str)
                )| {
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
                if self.selected == idx {
                    row = row.style(Style::default().bg(Color::Blue));
                }
                row
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
        ];

        Paragraph::new(right_footer_text)
            .block(right_footer_block)
            .render(footer_layout[2], buf);

    }

    fn render_search_popup(&self, area: Rect, buf: &mut Buffer) {
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
        if self.selected_guid.is_none() {
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

        let block = Block::new()
            .title("Details")
            .borders(Borders::ALL);

        let inner_area = block.inner(rect);
        let column_ratios = [0.0, 0.0, 0.5, 0.18, 0.18, 0.18, 0.18, 0.23];
        let widths = compute_column_widths(inner_area.width, &column_ratios);

        // Prepare node info
        let mut node_info: Vec<(i32, f64, f64, f64, u128, String)> = self
            .display_counters.clone()
            .into_iter()
            .map(|e,| {
                let recv_bw = get_bw(&e.1, "rcv_bytes", &self.counter_mode);
                let xmt_bw = get_bw(&e.1, "xmt_bytes", &self.counter_mode);
                let xmit_waits = get_bw_loss(&e.1, "xmit_waits", &self.counter_mode);
                let error_count = count_errors(&e.1);
                let error_strings =  get_error_strings(&e.1);
                (
                    e.0.1,
                    recv_bw,
                    xmt_bw,
                    xmit_waits,
                    error_count,
                    error_strings
                )
            })
            .collect();

        node_info.sort_by(|a, b| a.0.cmp(&b.0));

        let visible_rows = inner_area.height.saturating_sub(1) as usize;
        self.visible_rows.set(visible_rows);
        let offset = self.popup_table_offset.min(node_info.len().saturating_sub(visible_rows));

        let rows = node_info
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
            .map(|(idx, (
                port, r_bw, x_bw, waits, errs, err_str)
                )| {
                let mut row = Row::new(vec![
                    Cell::from(format!("{}", port)),
                    Cell::from(format!("{:.2}", r_bw)),
                    Cell::from(format!("{:.2}", x_bw)),
                    Cell::from(format!("{:.2}", waits)),
                    Cell::from(format!("{}", errs)),
                    Cell::from(truncate_fit(err_str, widths[7])),
                ]);
                if self.popup_selected == idx {
                    row = row.style(Style::default().bg(Color::Blue));
                }
                row
            });

        let header_cells = vec![
            Cell::from(format!("PT")),
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
