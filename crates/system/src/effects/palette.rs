//! Colors and small helpers shared by the effects. The values match the
//! approved mockup, which differs slightly from the kit's DIM/FAINT.

use super::rng::Rng;
use ratatui::style::Color;
pub use telmo_kit::theme::{BLUE, CYAN, FG, GREEN, MAGENTA, RED, YELLOW};

pub const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
pub const WHITE: Color = Color::Rgb(0xe9, 0xec, 0xff);
pub const FAINT: Color = Color::Rgb(0x2f, 0x33, 0x4d);
pub const HOT_WHITE: Color = Color::Rgb(0xff, 0xf4, 0xd6);
pub const ICE: Color = Color::Rgb(0xe6, 0xf6, 0xff);
pub const SNOW_BLUE: Color = Color::Rgb(0xd7, 0xec, 0xff);
pub const LIME: Color = Color::Rgb(0xd9, 0xf9, 0x9d);
pub const PINK: Color = Color::Rgb(0xf5, 0xd0, 0xfe);

const BACKGROUND: [f32; 3] = [22.0, 23.0, 34.0];
const GLYPHS: &[u8] = b"0123456789ABCDEF<>/\\|=+*#%&$@:;{}[]";

/// The apple's colors in band order, top to bottom.
pub fn rainbow() -> [Color; 6] {
    [GREEN, YELLOW, ORANGE, RED, MAGENTA, BLUE]
}

/// The single-letter color codes used by logos.json.
pub fn code_color(code: char) -> Color {
    match code {
        'r' => RED,
        'o' => ORANGE,
        'g' => GREEN,
        'y' => YELLOW,
        'b' => BLUE,
        'm' => MAGENTA,
        'c' => CYAN,
        _ => FG,
    }
}

pub fn glyph(rng: &mut Rng) -> char {
    GLYPHS[rng.below(GLYPHS.len())] as char
}

fn channels(color: Color) -> [f32; 3] {
    match color {
        Color::Rgb(r, g, b) => [r as f32, g as f32, b as f32],
        _ => [255.0; 3],
    }
}

/// Mixes toward the popup background: 1 is the full color, 0 is invisible.
pub fn shade(color: Color, t: f32) -> Color {
    let c = channels(color);
    let mix = |i: usize| round(BACKGROUND[i] + (c[i] - BACKGROUND[i]) * t).clamp(0, 255) as u8;
    Color::Rgb(mix(0), mix(1), mix(2))
}

/// Hue in degrees, saturation and lightness in 0..1.
pub fn hsl(h: f32, s: f32, l: f32) -> Color {
    let a = s * l.min(1.0 - l);
    let channel = |n: f32| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let v = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        round(v * 255.0).clamp(0, 255) as u8
    };
    Color::Rgb(channel(0.0), channel(8.0), channel(4.0))
}

/// Rounds halves up like JavaScript's Math.round (also for negatives).
pub fn round(x: f32) -> i32 {
    (x + 0.5).floor() as i32
}
