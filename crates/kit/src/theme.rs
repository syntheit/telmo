//! Tokyo Night, as semantic roles. Backgrounds are left to the terminal so
//! the popup's translucency shows through.

use ratatui::style::{Color, Modifier, Style};

pub const FG: Color = Color::Rgb(0xc0, 0xca, 0xf5);
pub const DIM: Color = Color::Rgb(0x73, 0x7a, 0xa2);
pub const FAINT: Color = Color::Rgb(0x3b, 0x42, 0x61);
pub const BLUE: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
pub const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
pub const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
pub const MAGENTA: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
pub const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
pub const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
pub const SELECTION: Color = Color::Rgb(0x28, 0x34, 0x57);
pub const MODAL: Color = Color::Rgb(0x13, 0x14, 0x1f);
pub const FIELD: Color = Color::Rgb(0x24, 0x28, 0x3b);

pub fn text() -> Style {
    Style::new().fg(FG)
}
pub fn bold() -> Style {
    text().add_modifier(Modifier::BOLD)
}
pub fn dim() -> Style {
    Style::new().fg(DIM)
}
pub fn faint() -> Style {
    Style::new().fg(FAINT)
}
pub fn accent() -> Style {
    Style::new().fg(BLUE)
}
pub fn ok() -> Style {
    Style::new().fg(GREEN)
}
pub fn warn() -> Style {
    Style::new().fg(YELLOW)
}
pub fn err() -> Style {
    Style::new().fg(RED)
}
pub fn info() -> Style {
    Style::new().fg(CYAN)
}
