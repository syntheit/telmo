//! A grid of colored characters the effects draw into. The UI copies it into
//! the ratatui buffer; empty cells stay transparent so the popup's tint shows.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bold: bool,
}

pub struct Canvas {
    pub width: u16,
    pub height: u16,
    cells: Vec<Option<Cell>>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Canvas {
        Canvas {
            width,
            height,
            cells: vec![None; width as usize * height as usize],
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(None);
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let inside = x >= 0 && y >= 0 && x < self.width as i32 && y < self.height as i32;
        inside.then(|| y as usize * self.width as usize + x as usize)
    }

    /// Out-of-bounds writes are ignored, so effects can draw freely near the edges.
    pub fn set(&mut self, x: i32, y: i32, ch: char, fg: Color, bold: bool) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = Some(Cell { ch, fg, bold });
        }
    }

    pub fn text(&mut self, x: i32, y: i32, text: &str, fg: Color, bold: bool) {
        for (i, ch) in text.chars().enumerate() {
            self.set(x + i as i32, y, ch, fg, bold);
        }
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Cell> {
        self.index(x, y).and_then(|i| self.cells[i])
    }
}
