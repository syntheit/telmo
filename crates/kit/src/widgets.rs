//! Small drawing helpers. Screens are built from these so every module looks
//! the same: rounded panes, a selection bar, a key bar, toasts and dialogs.

use crate::theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};
use std::time::{Duration, Instant};

/// Header, body, toast row and key bar.
pub struct Screen {
    pub header: Rect,
    pub body: Rect,
    pub toast: Rect,
    pub keys: Rect,
}

pub fn screen(area: Rect) -> Screen {
    let [header, _, body, toast, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    Screen {
        header,
        body,
        toast,
        keys,
    }
}

/// " Title" on the left, one status item on the right.
pub fn header(frame: &mut Frame, area: Rect, title: &str, status: Line) {
    frame.render_widget(
        Line::styled(format!(" {title}"), theme::accent().bold()),
        area,
    );
    let mut status = status;
    status.spans.push(Span::raw(" "));
    frame.render_widget(status.right_aligned(), area);
}

/// A rounded pane. `badge` is a dim count after the title; `right` is an
/// optional label on the right of the top border.
pub fn pane<'a>(
    title: &'a str,
    active: bool,
    badge: Option<String>,
    right: Option<Line<'a>>,
) -> Block<'a> {
    pane_colored(title, active, badge, right, None)
}

/// A pane drawn in a warning color, for banners like "names hidden".
pub fn warning_pane<'a>(title: &'a str) -> Block<'a> {
    pane_colored(title, false, None, None, Some(theme::YELLOW))
}

fn pane_colored<'a>(
    title: &'a str,
    active: bool,
    badge: Option<String>,
    right: Option<Line<'a>>,
    color: Option<ratatui::style::Color>,
) -> Block<'a> {
    let border = match color {
        Some(c) => Style::new().fg(c),
        None if active => theme::accent(),
        None => theme::faint(),
    };
    let title_style = match color {
        Some(c) => Style::new().fg(c).bold(),
        None if active => theme::accent().bold(),
        None => theme::bold(),
    };
    let mut spans = vec![
        Span::styled("─", border),
        Span::styled(format!(" {title} "), title_style),
    ];
    if let Some(badge) = badge {
        spans.push(Span::styled(format!("{badge} "), theme::dim()));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(spans));
    if let Some(mut right) = right {
        right.spans.insert(0, Span::raw(" "));
        right.spans.push(Span::raw(" "));
        right.spans.push(Span::styled("─", border));
        block = block.title(right.right_aligned());
    }
    block
}

/// Draw selectable rows. Each row gets the selection marker column; the
/// selected row gets a full-width background. Scrolls to keep it visible.
/// Returns the index and area of every row drawn, for click handling.
pub fn rows(
    frame: &mut Frame,
    area: Rect,
    lines: Vec<Line>,
    selected: Option<usize>,
) -> Vec<(usize, Rect)> {
    let mut drawn = Vec::new();
    let height = area.height as usize;
    let offset = match selected {
        Some(s) if s >= height => s + 1 - height,
        _ => 0,
    };
    for (i, line) in lines.into_iter().enumerate().skip(offset).take(height) {
        let rect = Rect {
            y: area.y + (i - offset) as u16,
            height: 1,
            ..area
        };
        drawn.push((i, rect));
        let is_selected = selected == Some(i);
        let marker = if is_selected {
            Span::styled(" ▌ ", theme::accent())
        } else {
            Span::raw("   ")
        };
        let mut line = line;
        line.spans.insert(0, marker);
        if is_selected {
            frame
                .buffer_mut()
                .set_style(rect, Style::new().bg(theme::SELECTION));
        }
        frame.render_widget(line, rect);
    }
    drawn
}

/// Pad or cut a string to exactly `width` columns.
pub fn fit(text: &str, width: usize) -> String {
    let mut out: String = text.chars().take(width).collect();
    let len = out.chars().count();
    out.extend(std::iter::repeat_n(' ', width - len));
    out
}

/// Four signal bars for a 0–100 strength.
pub fn signal(strength: u8) -> Vec<Span<'static>> {
    let level = match strength {
        75.. => 4,
        50.. => 3,
        25.. => 2,
        _ => 1,
    };
    let style = match level {
        3.. => theme::ok(),
        2 => theme::warn(),
        _ => theme::err(),
    };
    "▂▄▆█"
        .chars()
        .enumerate()
        .map(|(i, c)| {
            Span::styled(
                c.to_string(),
                if i < level { style } else { theme::faint() },
            )
        })
        .collect()
}

/// A volume-style bar: `━` filled, `─` empty. `fraction` is clamped to 0..=1.
pub fn gauge(fraction: f32, width: usize, style: Style) -> Vec<Span<'static>> {
    let filled = (fraction.clamp(0.0, 1.0) * width as f32).round() as usize;
    vec![
        Span::styled("━".repeat(filled), style),
        Span::styled("─".repeat(width - filled), theme::faint()),
    ]
}

/// A battery percentage colored by level.
pub fn battery(percent: u8) -> Span<'static> {
    let style = match percent {
        51.. => theme::ok(),
        21.. => theme::warn(),
        _ => theme::err(),
    };
    Span::styled(format!("{percent}%"), style)
}

const SPINNER: [&str; 6] = ["◜", "◠", "◝", "◞", "◡", "◟"];

pub fn spinner(tick: u64) -> &'static str {
    SPINNER[(tick % SPINNER.len() as u64) as usize]
}

/// Three-row digits for big readouts (speedtest, pairing codes).
pub fn big_digits(text: &str, style: Style) -> [Line<'static>; 3] {
    let mut rows = [String::new(), String::new(), String::new()];
    for c in text.chars() {
        let glyph: [&str; 3] = match c {
            '0' => ["█▀█", "█ █", "▀▀▀"],
            '1' => ["▀█ ", " █ ", "▀▀▀"],
            '2' => ["▀▀█", "█▀▀", "▀▀▀"],
            '3' => ["▀▀█", " ▀█", "▀▀▀"],
            '4' => ["█ █", "▀▀█", "  ▀"],
            '5' => ["█▀▀", "▀▀█", "▀▀▀"],
            '6' => ["█▀▀", "█▀█", "▀▀▀"],
            '7' => ["▀▀█", "  █", "  ▀"],
            '8' => ["█▀█", "█▀█", "▀▀▀"],
            '9' => ["█▀█", "▀▀█", "▀▀▀"],
            '.' => [" ", " ", "▀"],
            '-' => ["   ", "▀▀▀", "   "],
            _ => ["  ", "  ", "  "],
        };
        for (row, part) in rows.iter_mut().zip(glyph) {
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(part);
        }
    }
    rows.map(|r| Line::styled(r, style))
}

/// The bottom key bar: `key label  key label ...`. Returns the area of each
/// binding, in order, so a click can trigger the same action as the key.
pub fn keys(frame: &mut Frame, area: Rect, bindings: &[(&str, &str)]) -> Vec<Rect> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(*key, theme::accent()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*label, theme::dim()));
    }
    frame.render_widget(Line::from(spans), area);
    binding_areas(area, bindings, 1, 2)
}

/// Where each `key label` pair lands on a line that starts with `lead`
/// spaces and separates pairs with `gap` spaces.
fn binding_areas(area: Rect, bindings: &[(&str, &str)], lead: u16, gap: u16) -> Vec<Rect> {
    let mut x = area.x + lead;
    let mut out = Vec::new();
    for (key, label) in bindings {
        let width = (key.chars().count() + 1 + label.chars().count()) as u16;
        out.push(
            Rect {
                x,
                y: area.y,
                width,
                height: 1,
            }
            .intersection(area),
        );
        x += width + gap;
    }
    out
}

/// Areas of the bindings in a dialog `hint` line drawn at `area`.
pub fn hint_areas(area: Rect, bindings: &[(&str, &str)]) -> Vec<Rect> {
    binding_areas(area, bindings, 1, 3)
}

/// A short message above the key bar that expires on its own.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub ok: bool,
    pub until: Instant,
}

impl Toast {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ok: true,
            until: Instant::now() + Duration::from_secs(4),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ok: false,
            until: Instant::now() + Duration::from_secs(6),
        }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let (badge, text) = if self.ok {
            (
                Style::new().bg(theme::GREEN).fg(theme::MODAL).bold(),
                theme::ok(),
            )
        } else {
            (
                Style::new().bg(theme::RED).fg(theme::MODAL).bold(),
                theme::err(),
            )
        };
        let mark = if self.ok { " ✓ " } else { " ✗ " };
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(mark, badge),
            Span::raw(" "),
            Span::styled(self.message.clone(), text),
        ]);
        frame.render_widget(line, area);
    }
}

/// Dim everything already drawn, then draw a centered dialog of the given
/// outer size. Returns the inner area for the dialog's content.
pub fn dialog(frame: &mut Frame, title: &str, width: u16, height: u16) -> Rect {
    let full = frame.area();
    for cell in frame.buffer_mut().content.iter_mut() {
        cell.set_fg(theme::FAINT);
        cell.set_bg(ratatui::style::Color::Reset);
    }
    let width = width.min(full.width);
    let height = height.min(full.height);
    let area = Rect {
        x: full.x + (full.width - width) / 2,
        y: full.y + (full.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = pane(title, true, None, None).style(Style::new().bg(theme::MODAL));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Key hints inside a dialog, e.g. `↵ join   esc cancel`.
pub fn hint(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(key.to_string(), theme::accent()));
        spans.push(Span::styled(format!(" {label}"), theme::dim()));
    }
    Line::from(spans)
}

/// A paragraph of plain lines, for dialog bodies and empty states.
pub fn text<'a>(frame: &mut Frame, area: Rect, lines: Vec<Line<'a>>) {
    frame.render_widget(Paragraph::new(lines), area);
}
