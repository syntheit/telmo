//! Render a frame to plain text for snapshot tests.

use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{Frame, Terminal, backend::TestBackend};

pub fn render(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(draw).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Cell (column, row) of the first match of `text` on a rendered screen.
pub fn find(screen: &str, text: &str) -> Option<(u16, u16)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        let byte = line.find(text)?;
        Some((line[..byte].chars().count() as u16, row as u16))
    })
}

/// A mouse event at a cell, without modifiers.
pub fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cells_by_column_not_byte() {
        let screen = "ab\n→ x ok\n";
        assert_eq!(find(screen, "ok"), Some((4, 1)));
        assert_eq!(find(screen, "nope"), None);
    }
}
