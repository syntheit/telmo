//! The two logos (fastfetch's rainbow apple and the NixOS snowflake redrawn
//! at apple size), placed in a canvas and centered by visual weight.
//!
//! STUB: placement, mask and colors are filled in with the effects.

use crate::canvas::Canvas;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogoKind {
    Apple,
    Nix,
}

impl LogoKind {
    pub fn other(self) -> LogoKind {
        match self {
            LogoKind::Apple => LogoKind::Nix,
            LogoKind::Nix => LogoKind::Apple,
        }
    }

    /// The logo of the OS we run on.
    pub fn native() -> LogoKind {
        if cfg!(target_os = "macos") { LogoKind::Apple } else { LogoKind::Nix }
    }
}

pub struct LogoCell {
    /// Canvas position, before `offset`.
    pub x: i32,
    pub y: i32,
    /// Position inside the logo.
    pub lx: i32,
    pub ly: i32,
    pub ch: char,
    pub color: Color,
}

pub struct Logo {
    pub kind: LogoKind,
    pub cells: Vec<LogoCell>,
    /// Size of the logo itself.
    pub width: i32,
    pub height: i32,
    /// Top-left corner in the canvas.
    pub ox: i32,
    pub oy: i32,
    /// Moves the drawn logo (DVD bounces it); the mask does not follow.
    pub offset: (i32, i32),
    /// Canvas-sized: true where background effects must not draw (the logo's
    /// row spans, padded by one column each side).
    pub mask: Vec<bool>,
    canvas_width: u16,
}

impl Logo {
    pub fn place(kind: LogoKind, canvas_width: u16, canvas_height: u16) -> Logo {
        Logo {
            kind,
            cells: Vec::new(),
            width: 0,
            height: 0,
            ox: canvas_width as i32 / 2,
            oy: canvas_height as i32 / 2,
            offset: (0, 0),
            mask: vec![false; canvas_width as usize * canvas_height as usize],
            canvas_width,
        }
    }

    pub fn masked(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.canvas_width as i32
            && self.mask.get(y as usize * self.canvas_width as usize + x as usize).copied().unwrap_or(false)
    }

    pub fn draw(&self, canvas: &mut Canvas) {
        self.draw_with(canvas, |cell| cell.color);
    }

    pub fn draw_with(&self, canvas: &mut Canvas, color: impl Fn(&LogoCell) -> Color) {
        for cell in &self.cells {
            canvas.set(cell.x + self.offset.0, cell.y + self.offset.1, cell.ch, color(cell), true);
        }
    }
}
