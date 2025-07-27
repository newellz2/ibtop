use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{buffer::Buffer, 
    layout::{Constraint, Offset, Rect}, 
    style::{Color, Modifier, Style}, 
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget}
};

use crate::{ui::helpers::compute_column_widths};

#[derive(Debug)]
pub struct SearchForm {
    pub label: &'static str,
    pub value: String,
}

impl SearchForm {
    pub const fn new(label: &'static str) -> Self {
        Self {
            label,
            value: String::new(),
        }
    }

    /// Handle input events for the string input.
    pub fn on_key_press(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Char(c) => self.value.push(c),
            KeyCode::Backspace => {
                self.value.pop();
            }
            _ => {}
        }
    }

    pub fn cursor_offset(&self) -> Offset {
        let x = (self.value.len() + 1) as i32;
        Offset{
            x,
            y: 0,
        }
    }

}

impl Widget for &SearchForm {
    fn render(self, area: Rect, buf: &mut Buffer) {
    
        let block = Block::new()
        .title(self.label)
        .borders(Borders::ALL);

        let paragraph = Paragraph::new(
            self.value.clone()
        ).block(block);

        paragraph.render(area, buf);
    }
}

// Description Form
#[derive(Debug)]
pub struct NodeDetailsForm {
    pub label: &'static str,
}

impl NodeDetailsForm {
    pub const fn new(
        label: &'static str,
    ) -> Self {
        Self {
            label,
        }
    }

    /// Handle input events for the string input.
    pub fn on_key_press(&mut self, _event: KeyEvent) {
    }

    pub fn cursor_offset(&self) -> Offset {
        Offset{
            x: 0,
            y: 0,
        }
    }

}

impl Widget for &NodeDetailsForm {
    fn render(self, area: Rect, buf: &mut Buffer) {

        let block = Block::new()
            .title(self.label)
            .borders(Borders::ALL);

        let inner_area = block.inner(area);

        let column_ratios = [0.04, 0.32, 0.04, 0.12, 0.12, 0.12, 0.12, 0.12];
        let widths = compute_column_widths(inner_area.width, &column_ratios);

        let header_cells = vec![
            Cell::from(format!("LID")),
            Cell::from(format!("NODE")),
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
            Constraint::Length(widths[0] as u16),
            Constraint::Length(widths[1] as u16),
            Constraint::Length(widths[2] as u16),
            Constraint::Length(widths[3] as u16),
            Constraint::Length(widths[4] as u16),
            Constraint::Length(widths[5] as u16),
            Constraint::Length(widths[6] as u16),
            Constraint::Length(widths[7] as u16),
        ];

        let table = Table::new(vec![header.clone()], constraints)
            .header(header);

        table.render(inner_area, buf);
        
        block.render(area, buf);
    }
}