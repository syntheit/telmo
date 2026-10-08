//! The two logos (fastfetch's rainbow apple and the NixOS snowflake redrawn
//! at apple size), placed in a canvas and centered by visual weight.

use super::palette::{code_color, round};
use crate::canvas::Canvas;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        if cfg!(target_os = "macos") {
            LogoKind::Apple
        } else {
            LogoKind::Nix
        }
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
    canvas_height: u16,
}

impl Logo {
    /// Places the logo in a canvas of this size, centered by visual weight.
    pub fn place(kind: LogoKind, canvas_width: u16, canvas_height: u16) -> Logo {
        let rows = rows(kind);
        let width = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0) as i32;
        let height = rows.len() as i32;
        let (cw, ch) = (canvas_width as i32, canvas_height as i32);
        let (cx, cy) = weighted_center(&rows);
        let ox = round(cw as f32 / 2.0 - cx);
        let oy = round(ch as f32 / 2.0 - cy).min(ch - 1 - height).max(0);
        let mut logo = Logo {
            kind,
            cells: Vec::new(),
            width,
            height,
            ox,
            oy,
            offset: (0, 0),
            mask: vec![false; canvas_width as usize * canvas_height as usize],
            canvas_width,
            canvas_height,
        };
        for (ly, (text, codes)) in rows.iter().enumerate() {
            logo.add_row(ly as i32, text, codes);
        }
        logo
    }

    fn add_row(&mut self, ly: i32, text: &str, codes: &str) {
        let mut span: Option<(i32, i32)> = None;
        for (lx, (ch, code)) in text.chars().zip(codes.chars()).enumerate() {
            if ch == ' ' {
                continue;
            }
            let lx = lx as i32;
            let color = code_color(code);
            self.cells.push(LogoCell {
                x: self.ox + lx,
                y: self.oy + ly,
                lx,
                ly,
                ch,
                color,
            });
            span = Some(span.map_or((lx, lx), |(first, _)| (first, lx)));
        }
        if let Some((first, last)) = span {
            for lx in first - 1..=last + 1 {
                self.mask_cell(self.ox + lx, self.oy + ly);
            }
        }
    }

    fn mask_cell(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && x < self.canvas_width as i32 && y < self.canvas_height as i32 {
            self.mask[y as usize * self.canvas_width as usize + x as usize] = true;
        }
    }

    /// Size of the canvas this logo was placed in.
    pub fn canvas_size(&self) -> (i32, i32) {
        (self.canvas_width as i32, self.canvas_height as i32)
    }

    /// Middle of the logo's box, before `offset`.
    pub fn center(&self) -> (f32, f32) {
        (
            self.ox as f32 + self.width as f32 / 2.0,
            self.oy as f32 + self.height as f32 / 2.0,
        )
    }

    pub fn masked(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.canvas_width as i32
            && self
                .mask
                .get(y as usize * self.canvas_width as usize + x as usize)
                .copied()
                .unwrap_or(false)
    }

    pub fn draw(&self, canvas: &mut Canvas) {
        self.draw_with(canvas, |cell| cell.color);
    }

    pub fn draw_with(&self, canvas: &mut Canvas, color: impl Fn(&LogoCell) -> Color) {
        for cell in &self.cells {
            canvas.set(
                cell.x + self.offset.0,
                cell.y + self.offset.1,
                cell.ch,
                color(cell),
                true,
            );
        }
    }
}

/// Each row is its text and one color code per character.
fn rows(kind: LogoKind) -> Vec<(String, String)> {
    let key = match kind {
        LogoKind::Apple => "mac",
        LogoKind::Nix => "linux",
    };
    let mut all: HashMap<String, Vec<(String, String)>> =
        serde_json::from_str(include_str!("logos.json")).unwrap_or_default();
    all.remove(key).unwrap_or_default()
}

/// Block characters count for less than full ones, so the apple's thin leaf
/// doesn't push its body low.
fn weight(ch: char) -> f32 {
    match ch {
        '▘' | '▝' | '▖' | '▗' => 0.25,
        '▀' | '▄' | '▌' | '▐' | '▚' | '▞' => 0.5,
        '▛' | '▜' | '▙' | '▟' => 0.75,
        '█' => 1.0,
        c if c.is_ascii_alphanumeric() => 1.0,
        _ => 0.25,
    }
}

fn weighted_center(rows: &[(String, String)]) -> (f32, f32) {
    let (mut sx, mut sy, mut total) = (0.0, 0.0, 0.0);
    for (y, (text, _)) in rows.iter().enumerate() {
        for (x, ch) in text.chars().enumerate().filter(|(_, c)| *c != ' ') {
            let w = weight(ch);
            sx += w * (x as f32 + 0.5);
            sy += w * (y as f32 + 0.5);
            total += w;
        }
    }
    if total == 0.0 {
        (0.0, 0.0)
    } else {
        (sx / total, sy / total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_logos_fit_and_are_masked() {
        for kind in [LogoKind::Apple, LogoKind::Nix] {
            let logo = Logo::place(kind, 90, 21);
            assert!(!logo.cells.is_empty());
            assert!(logo.ox >= 0 && logo.oy >= 0);
            assert!(logo.ox + logo.width <= 90 && logo.oy + logo.height <= 21);
            for c in &logo.cells {
                assert!(logo.masked(c.x, c.y), "{kind:?} cell at {},{}", c.x, c.y);
            }
        }
    }

    #[test]
    fn small_canvas_does_not_panic() {
        for kind in [LogoKind::Apple, LogoKind::Nix] {
            let logo = Logo::place(kind, 40, 12);
            let mut canvas = Canvas::new(40, 12);
            logo.draw(&mut canvas);
            assert!(!logo.masked(-1, -1) && !logo.masked(500, 500));
        }
    }
}
