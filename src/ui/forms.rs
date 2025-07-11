use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{buffer::Buffer, layout::{Offset, Rect}, 
    widgets::{Block, Borders, Widget}};

#[derive(Debug)]
pub struct StringField {
    pub label: &'static str,
    pub value: String,
}

impl StringField {
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
            x: x,
            y: 0,
        }
    }

}

impl Widget for &StringField {
    fn render(self, area: Rect, buf: &mut Buffer) {
    
        let block = Block::new()
        .title(self.label)
        .borders(Borders::ALL);

        self.value.clone().render(block.inner(area), buf);
        block.render(area, buf);
    }
}
