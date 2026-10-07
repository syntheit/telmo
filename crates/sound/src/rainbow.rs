//! Bar colors: a rainbow across the width that slides over time, darker at
//! the base of a bar and lighter at its tip.

use ratatui::style::Color;

/// Hue degrees the gradient spans from the first bar to the last.
const SPAN: f32 = 300.0;
const SATURATION: f32 = 0.72;
/// Brightness at the base and at the top of a bar.
const BASE_VALUE: f32 = 0.60;
const TOP_VALUE: f32 = 0.98;
/// How far the tip cell and peak cap are mixed toward white.
const TIP_LIGHTEN: f32 = 0.35;
const CAP_LIGHTEN: f32 = 0.55;

/// `across` is 0.0-1.0 from left to right, `height` the cell's place in the
/// bar area from the bottom (0.0) to the top (1.0).
pub fn bar_color(across: f32, phase: f32, height: f32, tip: bool) -> Color {
    let value = BASE_VALUE + (TOP_VALUE - BASE_VALUE) * height.clamp(0.0, 1.0);
    let color = hsv(phase + across * SPAN, SATURATION, value);
    if tip {
        lighten(color, TIP_LIGHTEN)
    } else {
        color
    }
}

pub fn cap_color(across: f32, phase: f32) -> Color {
    lighten(
        hsv(phase + across * SPAN, SATURATION, TOP_VALUE),
        CAP_LIGHTEN,
    )
}

/// Hue in degrees (any value, it wraps), saturation and value 0.0-1.0.
pub fn hsv(hue: f32, saturation: f32, value: f32) -> Color {
    let h = hue.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    let byte = |c: f32| ((c + m) * 255.0).round() as u8;
    Color::Rgb(byte(r), byte(g), byte(b))
}

fn lighten(color: Color, amount: f32) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let up = |c: u8| (c as f32 + (255.0 - c as f32) * amount).round() as u8;
    Color::Rgb(up(r), up(g), up(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_hues() {
        assert_eq!(hsv(0.0, 1.0, 1.0), Color::Rgb(255, 0, 0));
        assert_eq!(hsv(120.0, 1.0, 1.0), Color::Rgb(0, 255, 0));
        assert_eq!(hsv(240.0, 1.0, 1.0), Color::Rgb(0, 0, 255));
        assert_eq!(hsv(360.0, 1.0, 1.0), hsv(0.0, 1.0, 1.0));
        assert_eq!(hsv(-120.0, 1.0, 1.0), hsv(240.0, 1.0, 1.0));
    }

    #[test]
    fn the_whole_width_is_a_rainbow() {
        let left = bar_color(0.0, 0.0, 1.0, false);
        let middle = bar_color(0.5, 0.0, 1.0, false);
        let right = bar_color(1.0, 0.0, 1.0, false);
        assert!(left != middle && middle != right && left != right);
    }

    #[test]
    fn drifting_moves_the_colors() {
        assert_ne!(
            bar_color(0.3, 0.0, 1.0, false),
            bar_color(0.3, 20.0, 1.0, false)
        );
        assert_eq!(
            bar_color(0.3, 0.0, 1.0, false),
            bar_color(0.3, 360.0, 1.0, false)
        );
    }

    fn brightness(color: Color) -> u32 {
        let Color::Rgb(r, g, b) = color else { return 0 };
        r as u32 + g as u32 + b as u32
    }

    #[test]
    fn bases_are_darker_and_tips_lighter() {
        assert!(
            brightness(bar_color(0.5, 0.0, 0.0, false))
                < brightness(bar_color(0.5, 0.0, 1.0, false))
        );
        assert!(
            brightness(bar_color(0.5, 0.0, 0.5, true))
                > brightness(bar_color(0.5, 0.0, 0.5, false))
        );
        assert!(brightness(cap_color(0.5, 0.0)) > brightness(bar_color(0.5, 0.0, 1.0, true)));
    }
}
