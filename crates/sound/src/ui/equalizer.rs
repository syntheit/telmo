//! The EQ screen of one output: its presets, and the curve they make.

use super::{key_hits, resample};
use crate::app::{App, Click, equalizer::EqView};
use crate::eq::{Choice, Filter, Preset, Resolved, Shape, resolve};
use crate::response::gain_db;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const SIDE_WIDTH: u16 = 26;
/// Columns left of the plot: the dB label, a space and the axis line.
const AXIS_WIDTH: usize = 5;
const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 20_000.0;
const RANGE_DB: f64 = 10.0;
/// The faint live spectrum fills this many rows at the bottom of the plot.
const SPECTRUM_ROWS: usize = 2;
const SPECTRUM_BARS: [char; 8] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇'];
const TICKS: [(f64, &str); 9] = [
    (20.0, "20"),
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1000.0, "1k"),
    (2000.0, "2k"),
    (5000.0, "5k"),
    (10_000.0, "10k"),
];
/// Braille dots by (row, column) in a cell, as bits.
const DOTS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

pub fn draw(app: &App, frame: &mut Frame, view: &EqView) {
    let screen = widgets::screen(frame.area());
    let Some(device) = app.eq_output(&view.device) else {
        return;
    };
    let Some(choice) = app.eq_choice(device) else {
        return;
    };
    let status = Line::styled(device.name.clone(), theme::dim());
    widgets::header(frame, screen.header, "Sound  ›  EQ", status);

    let [side, main] = Layout::horizontal([Constraint::Length(SIDE_WIDTH), Constraint::Fill(1)])
        .areas(screen.body);
    draw_presets(app, frame, side, view, &choice);
    draw_response(app, frame, main, &choice);

    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bass_label = format!("bass {:+} dB", choice.bass);
    let bindings = key_bar(app.eq.enabled, choice.bass, &bass_label);
    let areas = widgets::keys(frame, screen.keys, &bindings);
    key_hits(app, areas, &bindings, false);
}

fn key_bar(enabled: bool, bass: i32, bass_label: &str) -> Vec<(&str, &str)> {
    if !enabled {
        return vec![("b", "turn back on"), ("↑↓", "preset"), ("esc", "back")];
    }
    let (preset, on_off, back) = (("↑↓", "preset"), ("b", "on/off"), ("esc", "back"));
    if bass == 0 {
        return vec![preset, on_off, ("←→", "bass"), ("r", "reset"), back];
    }
    vec![("←→", bass_label), ("r", "reset"), preset, on_off, back]
}

fn draw_presets(app: &App, frame: &mut Frame, area: Rect, view: &EqView, choice: &Choice) {
    let block = widgets::pane("Presets", true, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = choice
        .presets()
        .iter()
        .enumerate()
        .map(|(i, preset)| {
            let dot = if i == choice.active {
                Span::styled("● ", Style::new().fg(theme::MAGENTA))
            } else {
                Span::raw("  ")
            };
            let name = if i == view.cursor {
                theme::bold()
            } else {
                theme::text()
            };
            Line::from(vec![dot, Span::styled(preset.name, name)])
        })
        .collect();
    for (i, rect) in widgets::rows(frame, inner, lines, Some(view.cursor)) {
        app.hits.add(rect, Click::Preset(i));
    }
}

fn draw_response(app: &App, frame: &mut Frame, area: Rect, choice: &Choice) {
    let enabled = app.eq.enabled;
    let preset = choice.preset();
    let resolved = resolve(preset, choice.bass);

    let mut title = preset.name.to_string();
    if choice.bass != 0 {
        title.push_str(&format!(" · bass {:+} dB", choice.bass));
    }
    if !enabled {
        title.push_str(" (off)");
    }
    let status = if enabled {
        Span::styled("on", theme::ok())
    } else {
        Span::styled("off", theme::err())
    };
    let block = widgets::pane(&title, false, None, Some(Line::from(status)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Under the curve: the filters, and one line for a warning.
    let plot_rows = (inner.height as usize).saturating_sub(3);
    let mut lines = curve(
        app,
        &resolved.filters,
        enabled,
        inner.width as usize,
        plot_rows,
    );
    lines.push(summary(preset, &resolved));
    lines.push(match preset.note {
        Some(note) => Line::styled(format!("  {note}"), theme::warn()),
        None => Line::raw(""),
    });
    widgets::text(frame, inner, lines);
}

/// "preamp -3 dB  ·  160 Hz +3", or a sentence when there's nothing to say.
fn summary(preset: &Preset, resolved: &Resolved) -> Line<'static> {
    if preset.filters.is_empty() && resolved.filters.is_empty() {
        return Line::styled(
            "  No filters: the sound as the device makes it.",
            theme::dim(),
        );
    }
    let mut spans = vec![Span::styled(
        format!("  preamp {} dB", resolved.preamp),
        theme::dim(),
    )];
    if !preset.filters.is_empty() {
        let filters: Vec<String> = preset.filters.iter().map(describe).collect();
        spans.push(Span::styled("  ·  ", theme::faint()));
        spans.push(Span::styled(filters.join(" · "), theme::dim()));
    }
    Line::from(spans)
}

fn describe(filter: &Filter) -> String {
    let freq = if filter.freq >= 1000.0 {
        format!("{}k", filter.freq / 1000.0)
    } else {
        format!("{}", filter.freq)
    };
    match filter.shape {
        Shape::Peak => format!("{freq} Hz {:+}", filter.gain),
        Shape::LowShelf | Shape::HighShelf => format!("shelf {freq} Hz {:+}", filter.gain),
        Shape::HighPass => format!("cut below {freq} Hz"),
    }
}

/// The response in braille on a log frequency axis, `RANGE_DB` up and down,
/// with the live spectrum faint behind it, and the frequency labels below.
fn curve(
    app: &App,
    filters: &[Filter],
    enabled: bool,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let plot = width.saturating_sub(AXIS_WIDTH + 2);
    if rows < 3 || plot < 2 {
        return Vec::new();
    }
    let dots = trace(filters, enabled, plot * 2, rows * 4);
    let zero = (((rows * 4 - 1) as f64 / 2.0).round() as usize) / 4;
    let labels = [
        (0, "+10"),
        (zero / 2, " +5"),
        (zero, "  0"),
        ((zero + rows - 1) / 2, " -5"),
        (rows - 1, "-10"),
    ];
    let spectrum = resample(&app.motion.bars, plot);
    let color = if enabled {
        theme::accent()
    } else {
        theme::dim()
    }
    .bold();
    let mut lines = Vec::new();
    for row in 0..rows {
        let label = labels.iter().find(|(r, _)| *r == row).map(|(_, l)| *l);
        let mut spans = vec![
            Span::raw(" "),
            Span::styled(format!("{:>3} ", label.unwrap_or("")), theme::dim()),
            Span::styled(if label.is_some() { "┤" } else { "│" }, theme::faint()),
        ];
        for col in 0..plot {
            spans.push(plot_cell(&dots, &spectrum, (row, col), rows, zero, color));
        }
        lines.push(Line::from(spans));
    }
    lines.push(axis(width, plot));
    lines
}

/// Which dots are lit: one per column, joined to the last so a steep part of
/// the curve has no gaps.
fn trace(filters: &[Filter], enabled: bool, columns: usize, height: usize) -> Vec<Vec<bool>> {
    let mut dots = vec![vec![false; columns]; height];
    let mut previous: Option<usize> = None;
    for x in 0..columns {
        let hz = MIN_HZ * (MAX_HZ / MIN_HZ).powf(x as f64 / (columns - 1) as f64);
        let db = if enabled { gain_db(filters, hz) } else { 0.0 };
        let level = (RANGE_DB - db.clamp(-RANGE_DB, RANGE_DB)) / (2.0 * RANGE_DB);
        let y = (level * (height - 1) as f64).round() as usize;
        let (low, high) = previous.map_or((y, y), |p| (p.min(y), p.max(y)));
        for row in dots.iter_mut().take(high + 1).skip(low) {
            row[x] = true;
        }
        previous = Some(y);
    }
    dots
}

fn plot_cell(
    dots: &[Vec<bool>],
    spectrum: &[f32],
    (row, col): (usize, usize),
    rows: usize,
    zero: usize,
    color: Style,
) -> Span<'static> {
    let mut bits = 0;
    for (dy, line) in DOTS.iter().enumerate() {
        for (dx, bit) in line.iter().enumerate() {
            if dots[row * 4 + dy][col * 2 + dx] {
                bits |= bit;
            }
        }
    }
    if bits != 0 {
        let glyph = char::from_u32(0x2800 + bits).unwrap_or(' ');
        return Span::styled(glyph.to_string(), color);
    }
    let from_bottom = rows - 1 - row;
    let height = f64::from(spectrum[col]) * 2.2;
    if from_bottom < SPECTRUM_ROWS && height > from_bottom as f64 {
        let part = (height - from_bottom as f64).min(1.0);
        let bar = SPECTRUM_BARS[((part * 7.0).round() as usize).max(1)];
        return Span::styled(bar.to_string(), Style::new().fg(theme::SELECTION));
    }
    if row == zero {
        return Span::styled("─", theme::faint());
    }
    Span::raw(" ")
}

/// Frequency labels under the plot, each starting at its tick.
fn axis(width: usize, plot: usize) -> Line<'static> {
    let mut cells = vec![' '; width];
    for (hz, label) in TICKS {
        let at = (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln();
        let x = 1 + AXIS_WIDTH + (at * (plot - 1) as f64).round() as usize;
        for (i, c) in label.chars().enumerate() {
            if let Some(cell) = cells.get_mut(x + i) {
                *cell = c;
            }
        }
    }
    Line::styled(cells.into_iter().collect::<String>(), theme::dim())
}
