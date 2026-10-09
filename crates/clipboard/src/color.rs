//! Colors copied as text: parsing and the three notations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notation {
    Hex,
    Rgb,
    Hsl,
}

impl Notation {
    pub const ALL: [Notation; 3] = [Notation::Hex, Notation::Rgb, Notation::Hsl];

    pub fn name(self) -> &'static str {
        match self {
            Notation::Hex => "hex",
            Notation::Rgb => "rgb",
            Notation::Hsl => "hsl",
        }
    }

    /// The notation a color string is written in.
    pub fn of(text: &str) -> Option<Notation> {
        let text = text.trim().to_ascii_lowercase();
        if text.starts_with('#') {
            Some(Notation::Hex)
        } else if text.starts_with("rgb") {
            Some(Notation::Rgb)
        } else if text.starts_with("hsl") {
            Some(Notation::Hsl)
        } else {
            None
        }
    }

    /// The notation `steps` places after this one, wrapping around.
    pub fn after(self, steps: usize) -> Notation {
        let at = Self::ALL.iter().position(|n| *n == self).unwrap_or(0);
        Self::ALL[(at + steps) % 3]
    }
}

impl Rgb {
    pub fn parse(text: &str) -> Option<Rgb> {
        let text = text.trim().to_ascii_lowercase();
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex);
        }
        if let Some(args) = function_args(&text, "rgb") {
            let [r, g, b] = numbers::<3>(&args, 255.0, false)?;
            return Some(Rgb(r as u8, g as u8, b as u8));
        }
        let args = function_args(&text, "hsl")?;
        let [h, s, l] = numbers::<3>(&args, 100.0, true)?;
        Some(hsl_to_rgb(h, s / 100.0, l / 100.0))
    }

    pub fn format(self, notation: Notation) -> String {
        match notation {
            Notation::Hex => self.hex(),
            Notation::Rgb => self.rgb(),
            Notation::Hsl => self.hsl(),
        }
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    pub fn rgb(self) -> String {
        format!("rgb({}, {}, {})", self.0, self.1, self.2)
    }

    pub fn hsl(self) -> String {
        let (h, s, l) = self.to_hsl();
        format!(
            "hsl({}, {}%, {}%)",
            h.round(),
            (s * 100.0).round(),
            (l * 100.0).round()
        )
    }

    fn to_hsl(self) -> (f64, f64, f64) {
        let [r, g, b] = [self.0, self.1, self.2].map(|c| f64::from(c) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d == 0.0 {
            return (0.0, 0.0, l);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        ((h * 60.0).rem_euclid(360.0), s, l)
    }
}

fn parse_hex(hex: &str) -> Option<Rgb> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some(Rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
        6 => Some(Rgb(pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}

/// The text between the parentheses of `name(...)`.
fn function_args(text: &str, name: &str) -> Option<String> {
    let rest = text.strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.to_string())
}

/// Exactly `N` numbers separated by commas or spaces. A trailing `%` is
/// allowed (and required to be absent on rgb); `degrees` allows `deg`.
fn numbers<const N: usize>(args: &str, max: f64, hsl: bool) -> Option<[f64; N]> {
    let parts: Vec<&str> = args.split([',', ' ']).filter(|p| !p.is_empty()).collect();
    if parts.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (i, part) in parts.iter().enumerate() {
        let part = part.trim_end_matches("deg");
        let part = if hsl && i > 0 {
            part.trim_end_matches('%')
        } else {
            part
        };
        let value: f64 = part.parse().ok()?;
        let limit = if hsl && i == 0 { 360.0 } else { max };
        if !(0.0..=limit).contains(&value) {
            return None;
        }
        out[i] = value;
    }
    Some(out)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round() as u8;
    Rgb(byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_notations() {
        assert_eq!(Rgb::parse("#7aa2f7"), Some(Rgb(122, 162, 247)));
        assert_eq!(Rgb::parse("#fff"), Some(Rgb(255, 255, 255)));
        assert_eq!(Rgb::parse(" rgb(122, 162, 247) "), Some(Rgb(122, 162, 247)));
        assert_eq!(Rgb::parse("rgb(122 162 247)"), Some(Rgb(122, 162, 247)));
        assert_eq!(Rgb::parse("hsl(0, 100%, 50%)"), Some(Rgb(255, 0, 0)));
    }

    #[test]
    fn rejects_things_that_are_not_colors() {
        for text in [
            "#12",
            "#ggg",
            "rgb(1,2)",
            "rgb(256,0,0)",
            "hsl(400, 1%, 1%)",
            "red",
            "#1234567",
        ] {
            assert_eq!(Rgb::parse(text), None, "{text}");
        }
    }

    #[test]
    fn converts_between_notations() {
        let c = Rgb(122, 162, 247);
        assert_eq!(c.hex(), "#7aa2f7");
        assert_eq!(c.rgb(), "rgb(122, 162, 247)");
        assert_eq!(c.hsl(), "hsl(221, 89%, 72%)");
        assert_eq!(Rgb(0, 0, 0).hsl(), "hsl(0, 0%, 0%)");
        assert_eq!(Rgb(255, 0, 0).hsl(), "hsl(0, 100%, 50%)");
        assert_eq!(Rgb(0, 0, 255).hsl(), "hsl(240, 100%, 50%)");
    }

    #[test]
    fn hsl_round_trips_for_primaries() {
        for c in [
            Rgb(255, 0, 0),
            Rgb(0, 255, 0),
            Rgb(0, 0, 255),
            Rgb(128, 128, 128),
        ] {
            assert_eq!(Rgb::parse(&c.hsl()), Some(c));
        }
    }

    #[test]
    fn notation_cycles() {
        assert_eq!(Notation::of("#fff"), Some(Notation::Hex));
        assert_eq!(Notation::Hex.after(1), Notation::Rgb);
        assert_eq!(Notation::Hsl.after(1), Notation::Hex);
        assert_eq!(Notation::of("blue"), None);
    }
}
