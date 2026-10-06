//! A single-line text field (password or plain). The cursor stays at the end.

use crate::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{style::Style, text::Span};

#[derive(Debug, Default, Clone)]
pub struct TextInput {
    pub value: String,
    pub masked: bool,
}

impl TextInput {
    pub fn masked() -> Self {
        Self {
            value: String::new(),
            masked: true,
        }
    }

    pub fn with_value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            masked: false,
        }
    }

    /// Returns true when the key edited the text.
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.value.clear()
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let trimmed = self.value.trim_end().len();
                let cut = self.value[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                self.value.truncate(cut);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.value.push(c)
            }
            KeyCode::Backspace => {
                self.value.pop();
            }
            _ => return false,
        }
        true
    }

    /// The field as spans of exactly `width` cells, with a cursor when focused.
    pub fn spans(&self, width: usize, focused: bool) -> Vec<Span<'static>> {
        let field = Style::new().bg(theme::FIELD);
        let shown: String = if self.masked {
            "•".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        };
        let room = width.saturating_sub(2);
        let visible: String = {
            let chars: Vec<char> = shown.chars().collect();
            chars[chars.len().saturating_sub(room)..].iter().collect()
        };
        let used = visible.chars().count() + 1;
        let mut spans = vec![Span::styled(format!(" {visible}"), field.fg(theme::FG))];
        if focused {
            spans.push(Span::styled("▏", field.fg(theme::BLUE)));
        } else {
            spans.push(Span::styled(" ", field));
        }
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used + 1)),
            field,
        ));
        spans
    }
}
