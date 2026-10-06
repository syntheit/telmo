//! Drawing only: a pure function of the app state.

use crate::app::{App, Dialog, Pane};
use crate::model::Device;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const NAME_WIDTH: usize = 30;
const GAUGE_WIDTH: usize = 30;

pub fn draw(app: &App, frame: &mut Frame) {
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Sound", Line::default());
    draw_panes(app, frame, screen.body);
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    widgets::keys(frame, screen.keys, &key_bar(app));
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, dialog, frame);
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    let action = match app.pane {
        Pane::Playing => ("o", "output"),
        _ => ("↵", "set default"),
    };
    vec![
        ("←→", "volume"),
        ("m", "mute"),
        action,
        ("tab", "pane"),
        ("?", "more"),
        ("esc", "close"),
    ]
}

fn draw_panes(app: &App, frame: &mut Frame, body: Rect) {
    let panes = app.panes();
    let heights = panes
        .iter()
        .map(|p| Constraint::Length(row_count(app, *p) as u16 + 2));
    let areas = Layout::vertical(heights.chain([Constraint::Fill(1)])).split(body);
    for (pane, area) in panes.into_iter().zip(areas.iter()) {
        draw_pane(app, frame, *area, pane);
    }
}

fn row_count(app: &App, pane: Pane) -> usize {
    let len = match pane {
        Pane::Playing => app.snapshot.streams.len(),
        _ => app.devices(pane).len(),
    };
    len.max(1)
}

fn draw_pane(app: &App, frame: &mut Frame, area: Rect, pane: Pane) {
    let title = match pane {
        Pane::Output => "Output",
        Pane::Input => "Input",
        Pane::Playing => "Playing",
    };
    let active = app.pane == pane;
    let block = widgets::pane(title, active, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = match pane {
        Pane::Playing => stream_lines(app),
        _ => app.devices(pane).iter().map(device_line).collect(),
    };
    if lines.is_empty() {
        let empty = match pane {
            Pane::Playing => "Nothing is playing.",
            Pane::Input => "No input devices.",
            Pane::Output => "No output devices.",
        };
        let line = Line::styled(format!("   {empty}"), theme::dim());
        widgets::text(frame, inner, vec![line]);
        return;
    }
    widgets::rows(frame, inner, lines, active.then(|| app.selected(pane)));
}

fn device_line(device: &Device) -> Line<'static> {
    let dot = if device.default {
        Span::styled("● ", theme::accent())
    } else {
        Span::raw("  ")
    };
    level_line(dot, &device.name, device.volume, device.muted, None)
}

fn stream_lines(app: &App) -> Vec<Line<'static>> {
    let default = app.default_output();
    app.snapshot
        .streams
        .iter()
        .map(|stream| {
            let elsewhere = stream
                .device
                .as_ref()
                .filter(|d| Some(*d) != default.as_ref());
            let target = elsewhere.and_then(|id| app.snapshot.outputs.iter().find(|d| &d.id == id));
            let note = target.map(|d| format!("→ {}", d.name));
            level_line(
                Span::raw("  "),
                &stream.app,
                Some(stream.volume),
                stream.muted,
                note,
            )
        })
        .collect()
}

/// `dot name gauge percent  note`; the gauge is replaced by "fixed volume"
/// when there is no software volume.
fn level_line(
    dot: Span<'static>,
    name: &str,
    volume: Option<f32>,
    muted: bool,
    note: Option<String>,
) -> Line<'static> {
    let mut spans = vec![
        dot,
        Span::styled(widgets::fit(name, NAME_WIDTH), theme::text()),
    ];
    match volume {
        None => spans.push(Span::styled("fixed volume", theme::faint())),
        Some(volume) => {
            let style = if muted { theme::dim() } else { theme::accent() };
            spans.extend(widgets::gauge(volume, GAUGE_WIDTH, style));
            spans.push(if muted {
                Span::styled(format!("{:>7}", "muted"), theme::err())
            } else {
                let percent = format!("{}%", (volume * 100.0).round() as u32);
                Span::styled(format!("{percent:>7}"), theme::text())
            });
        }
    }
    if let Some(note) = note {
        spans.push(Span::styled(format!("  {note}"), theme::dim()));
    }
    Line::from(spans)
}

fn draw_dialog(app: &App, dialog: &Dialog, frame: &mut Frame) {
    match dialog {
        Dialog::Help => draw_help(frame),
        Dialog::Route { stream, selected } => draw_route(app, frame, stream, *selected),
        Dialog::Profile { device, selected } => draw_profile(app, frame, device, *selected),
        Dialog::ProfileInfo { device } => draw_profile_info(app, frame, device),
    }
}

fn padded(hint: Line<'static>) -> Line<'static> {
    let mut hint = hint;
    hint.spans.insert(0, Span::raw(" "));
    hint
}

/// A dialog body: one blank row, the choices, a blank row, the hint line.
fn pick_list(
    frame: &mut Frame,
    inner: Rect,
    lines: Vec<Line<'static>>,
    selected: usize,
    hint: Line<'static>,
) {
    let count = lines.len() as u16;
    let list = Rect {
        y: inner.y + 1,
        height: count,
        ..inner
    };
    widgets::rows(frame, list, lines, Some(selected));
    let hint_row = Rect {
        y: inner.y + count + 2,
        height: 1,
        ..inner
    };
    frame.render_widget(padded(hint), hint_row);
}

fn marked(active: bool, name: &str) -> Line<'static> {
    let dot = if active {
        Span::styled("● ", theme::accent())
    } else {
        Span::raw("  ")
    };
    Line::from(vec![dot, Span::styled(name.to_string(), theme::text())])
}

fn draw_route(app: &App, frame: &mut Frame, stream: &str, selected: usize) {
    let Some(stream) = app.snapshot.streams.iter().find(|s| s.id == stream) else {
        return;
    };
    let current = stream.device.clone().or_else(|| app.default_output());
    let lines: Vec<_> = app
        .snapshot
        .outputs
        .iter()
        .map(|d| marked(Some(&d.id) == current.as_ref(), &d.name))
        .collect();
    let height = lines.len() as u16 + 6;
    let title = format!("Play {} on", stream.app);
    let inner = widgets::dialog(frame, &title, 50, height);
    let hint = widgets::hint(&[("↵", "move"), ("esc", "cancel")]);
    pick_list(frame, inner, lines, selected, hint);
}

fn draw_profile(app: &App, frame: &mut Frame, device: &str, selected: usize) {
    let Some(device) = app.snapshot.outputs.iter().find(|d| d.id == device) else {
        return;
    };
    let lines: Vec<_> = device
        .profiles
        .iter()
        .map(|p| marked(p.active, &p.name))
        .collect();
    let inner = widgets::dialog(
        frame,
        &format!("{} mode", device.name),
        56,
        lines.len() as u16 + 6,
    );
    let hint = widgets::hint(&[("↵", "switch"), ("esc", "cancel")]);
    pick_list(frame, inner, lines, selected, hint);
}

fn draw_profile_info(app: &App, frame: &mut Frame, device: &str) {
    let name = app
        .snapshot
        .outputs
        .iter()
        .find(|d| d.id == device)
        .map_or("This device", |d| d.name.as_str());
    let inner = widgets::dialog(frame, &format!("{name} mode"), 56, 8);
    let body = vec![
        Line::raw(""),
        Line::styled(
            "  macOS switches this on its own: picking AirPods",
            theme::text(),
        ),
        Line::styled("  as the input turns on headset mode.", theme::text()),
        Line::raw(""),
        padded(widgets::hint(&[("esc", "back")])),
    ];
    widgets::text(frame, inner, body);
}

fn draw_help(frame: &mut Frame) {
    const KEYS: [(&str, &str); 10] = [
        ("tab", "switch pane"),
        ("j k", "move up and down"),
        ("h l", "volume down and up by 5%"),
        ("m", "mute or unmute"),
        ("↵", "make the device the default"),
        ("o", "move the app to another output"),
        ("P", "Bluetooth mode (best sound or headset)"),
        ("?", "this help"),
        ("esc", "close a dialog, or quit"),
        ("q", "quit"),
    ];
    let inner = widgets::dialog(frame, "Keys", 60, KEYS.len() as u16 + 2);
    let lines = KEYS
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("  {key:<5}"), theme::accent()),
                Span::styled(*what, theme::dim()),
            ])
        })
        .collect();
    widgets::text(frame, inner, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock;
    use crate::model::Snapshot;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use std::{cell::RefCell, rc::Rc};
    use telmo_kit::{App as _, runtime::KeyEvent};

    fn app(snapshot: Snapshot, keys: &str) -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Some(snapshot), tx, Rc::new(RefCell::new(None)));
        for c in keys.chars() {
            let code = match c {
                '\t' => KeyCode::Tab,
                '\n' => KeyCode::Enter,
                c => KeyCode::Char(c),
            };
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        app
    }

    fn render(app: &App) -> String {
        telmo_kit::test::render(90, 22, |f| draw(app, f))
    }

    #[test]
    fn mixer_linux() {
        insta::assert_snapshot!(render(&app(mock::linux(), "j")));
    }

    #[test]
    fn apps_linux() {
        insta::assert_snapshot!(render(&app(mock::linux(), "\t\tj")));
    }

    #[test]
    fn route_linux() {
        insta::assert_snapshot!(render(&app(mock::linux(), "\t\tjo")));
    }

    #[test]
    fn profile_linux() {
        insta::assert_snapshot!(render(&app(mock::linux(), "jP")));
    }

    #[test]
    fn mixer_mac() {
        insta::assert_snapshot!(render(&app(mock::mac(), "j")));
    }

    #[test]
    fn profile_mac() {
        insta::assert_snapshot!(render(&app(mock::mac(), "jP")));
    }
}
