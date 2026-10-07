//! Drawing only: a pure function of the app state.

use crate::app::{App, Click, Dialog, Pane};
use crate::model::Device;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const NAME_WIDTH: usize = 25;
/// Room after the name for the equalizer of a playing row.
const EQ_SLOT: usize = 5;
const GAUGE_WIDTH: usize = 30;
const EQ_BARS: usize = 4;
/// Columns before the gauge: selection marker, dot, name and equalizer slot.
const GAUGE_X: u16 = (3 + 2 + NAME_WIDTH + EQ_SLOT) as u16;

/// A small equalizer. Each bar rises and falls on its own period, so the
/// pattern looks irregular but only depends on the frame counter.
fn equalizer(frame: u64) -> Vec<Span<'static>> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const PERIODS: [u64; EQ_BARS] = [10, 14, 8, 12];
    (0..EQ_BARS)
        .map(|i| {
            let period = PERIODS[i];
            let phase = (frame + 3 * i as u64) % period;
            let half = period / 2;
            let up = if phase < half { phase } else { period - phase };
            let level = (up * 7 / half) as usize;
            Span::styled(LEVELS[level].to_string(), theme::accent())
        })
        .collect()
}

fn key_code(key: &str) -> Option<KeyCode> {
    match key {
        "↵" => Some(KeyCode::Enter),
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(KeyCode::Char(c)),
                _ => None,
            }
        }
    }
}

fn key_hits(app: &App, areas: Vec<Rect>, bindings: &[(&str, &str)], dialog: bool) {
    for (area, (key, _)) in areas.into_iter().zip(bindings) {
        if let Some(code) = key_code(key) {
            let click = if dialog {
                Click::DialogKey(code)
            } else {
                Click::Key(code)
            };
            app.hits.add(area, click);
        }
    }
}

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    let screen = widgets::screen(frame.area());
    let status = if app.playing() {
        Line::from(equalizer(app.frame))
    } else {
        Line::default()
    };
    widgets::header(frame, screen.header, "Sound", status);
    draw_panes(app, frame, screen.body);
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bindings = key_bar(app);
    let areas = widgets::keys(frame, screen.keys, &bindings);
    key_hits(app, areas, &bindings, false);
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
    app.hits.add(area, Click::Pane(pane));

    let lines = match pane {
        Pane::Playing => stream_lines(app),
        _ => app
            .devices(pane)
            .iter()
            .map(|d| device_line(d, app.frame))
            .collect(),
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
    let drawn = widgets::rows(frame, inner, lines, active.then(|| app.selected(pane)));
    for (i, rect) in drawn {
        app.hits.add(rect, Click::Row(pane, i));
        if has_volume(app, pane, i) {
            gauge_hits(app, rect, pane, i);
        }
    }
}

fn has_volume(app: &App, pane: Pane, i: usize) -> bool {
    match pane {
        Pane::Playing => i < app.snapshot.streams.len(),
        _ => app.devices(pane).get(i).is_some_and(|d| d.volume.is_some()),
    }
}

/// One hit per gauge cell; the first cell is 0% and the last 100%.
fn gauge_hits(app: &App, row: Rect, pane: Pane, i: usize) {
    for cell in 0..GAUGE_WIDTH {
        let rect = Rect {
            x: row.x + GAUGE_X + cell as u16,
            width: 1,
            ..row
        }
        .intersection(row);
        let volume = cell as f32 / (GAUGE_WIDTH - 1) as f32;
        app.hits.add(rect, Click::Volume(pane, i, volume));
    }
}

fn device_line(device: &Device, frame: u64) -> Line<'static> {
    let dot = if device.default {
        Span::styled("● ", theme::accent())
    } else {
        Span::raw("  ")
    };
    let eq = device.playing.then(|| equalizer(frame));
    level_line(dot, &device.name, eq, device.volume, device.muted, None)
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
            let eq = stream.playing.then(|| equalizer(app.frame));
            level_line(
                Span::raw("  "),
                &stream.app,
                eq,
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
    eq: Option<Vec<Span<'static>>>,
    volume: Option<f32>,
    muted: bool,
    note: Option<String>,
) -> Line<'static> {
    let mut spans = vec![
        dot,
        Span::styled(widgets::fit(name, NAME_WIDTH), theme::text()),
    ];
    spans.extend(eq.unwrap_or_else(|| vec![Span::raw(" ".repeat(EQ_BARS))]));
    spans.push(Span::raw(" "));
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

/// A click outside the dialog closes it; a click inside is not "outside".
fn dialog_hits(app: &App, frame: &Frame, inner: Rect) {
    app.hits.add(frame.area(), Click::Outside);
    let outer = Rect {
        x: inner.x.saturating_sub(1),
        y: inner.y.saturating_sub(1),
        width: inner.width + 2,
        height: inner.height + 2,
    };
    app.hits.add(outer, Click::Dialog);
}

fn draw_dialog(app: &App, dialog: &Dialog, frame: &mut Frame) {
    match dialog {
        Dialog::Help => draw_help(app, frame),
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
    app: &App,
    frame: &mut Frame,
    inner: Rect,
    lines: Vec<Line<'static>>,
    selected: usize,
    bindings: &[(&str, &str)],
) {
    let count = lines.len() as u16;
    let list = Rect {
        y: inner.y + 1,
        height: count,
        ..inner
    };
    for (i, rect) in widgets::rows(frame, list, lines, Some(selected)) {
        app.hits.add(rect, Click::DialogRow(i));
    }
    let hint_row = Rect {
        y: inner.y + count + 2,
        height: 1,
        ..inner
    };
    frame.render_widget(padded(widgets::hint(bindings)), hint_row);
    hint_hits(app, hint_row, bindings);
}

/// The hint line is drawn one column further in than `widgets::hint` alone.
fn hint_hits(app: &App, row: Rect, bindings: &[(&str, &str)]) {
    let row = Rect {
        x: row.x + 1,
        ..row
    };
    key_hits(app, widgets::hint_areas(row, bindings), bindings, true);
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
    dialog_hits(app, frame, inner);
    pick_list(
        app,
        frame,
        inner,
        lines,
        selected,
        &[("↵", "move"), ("esc", "cancel")],
    );
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
    dialog_hits(app, frame, inner);
    pick_list(
        app,
        frame,
        inner,
        lines,
        selected,
        &[("↵", "switch"), ("esc", "cancel")],
    );
}

fn draw_profile_info(app: &App, frame: &mut Frame, device: &str) {
    let name = app
        .snapshot
        .outputs
        .iter()
        .find(|d| d.id == device)
        .map_or("This device", |d| d.name.as_str());
    let inner = widgets::dialog(frame, &format!("{name} mode"), 56, 8);
    dialog_hits(app, frame, inner);
    let back = [("esc", "back")];
    let hint_row = Rect {
        y: inner.y + 4,
        height: 1,
        ..inner
    };
    hint_hits(app, hint_row, &back);
    let body = vec![
        Line::raw(""),
        Line::styled(
            "  macOS switches this on its own: picking AirPods",
            theme::text(),
        ),
        Line::styled("  as the input turns on headset mode.", theme::text()),
        Line::raw(""),
        padded(widgets::hint(&back)),
    ];
    widgets::text(frame, inner, body);
}

fn draw_help(app: &App, frame: &mut Frame) {
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
    dialog_hits(app, frame, inner);
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
    use telmo_kit::{
        Flow,
        runtime::{MouseButton, MouseEvent, MouseEventKind},
    };

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

    fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) -> Flow {
        render(app);
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(event)
    }

    fn click(app: &mut App, column: u16, row: u16) -> Flow {
        mouse(app, MouseEventKind::Down(MouseButton::Left), column, row)
    }

    /// Screen row of the first device row of a pane (header, blank, border).
    const FIRST_ROW: u16 = 3;

    fn spotify_volume(app: &App) -> f32 {
        app.snapshot.streams[0].volume
    }

    #[test]
    fn click_selects_then_sets_default() {
        let mut app = app(mock::linux(), "");
        click(&mut app, 10, FIRST_ROW + 1);
        assert_eq!(app.selected(Pane::Output), 1);
        assert!(!app.snapshot.outputs[1].default);
        click(&mut app, 10, FIRST_ROW + 1);
        assert!(app.snapshot.outputs[1].default);
    }

    #[test]
    fn click_focuses_pane() {
        let mut app = app(mock::linux(), "");
        // Input pane row: outputs 4 rows + 2 borders after the body start.
        click(&mut app, 10, 2 + 6 + 1);
        assert_eq!(app.pane, Pane::Input);
    }

    #[test]
    fn click_gauge_sets_volume() {
        let mut app = app(mock::linux(), "\t\t");
        let row = 2 + 6 + 3 + 1;
        let x = 1 + GAUGE_X;
        click(&mut app, x, row);
        assert_eq!(app.pane, Pane::Playing);
        assert_eq!(spotify_volume(&app), 0.0);
        click(&mut app, x + GAUGE_WIDTH as u16 - 1, row);
        assert_eq!(spotify_volume(&app), 1.0);
    }

    #[test]
    fn scroll_changes_volume_or_moves_selection() {
        let mut app = app(mock::linux(), "\t\t");
        let row = 2 + 6 + 3 + 1;
        mouse(&mut app, MouseEventKind::ScrollUp, 10, row);
        assert_eq!(spotify_volume(&app), 0.85);
        mouse(&mut app, MouseEventKind::ScrollDown, 10, row);
        mouse(&mut app, MouseEventKind::ScrollDown, 10, row);
        assert_eq!(spotify_volume(&app), 0.75);
        mouse(&mut app, MouseEventKind::ScrollDown, 85, 1);
        assert_eq!(app.selected(Pane::Playing), 1);
    }

    #[test]
    fn click_key_bar_and_dialog() {
        let mut app = app(mock::linux(), "");
        // "?" is the second to last item on the key bar.
        let bar = render(&app);
        let last = bar.lines().nth(21).unwrap_or_default().to_string();
        let x = last
            .find('?')
            .map(|b| last[..b].chars().count())
            .unwrap_or(0);
        click(&mut app, x as u16, 21);
        assert!(matches!(app.dialog, Some(Dialog::Help)));
        // Inside the dialog does nothing, outside closes it.
        click(&mut app, 45, 11);
        assert!(app.dialog.is_some());
        click(&mut app, 1, 1);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn click_dialog_row_then_confirm() {
        let mut app = app(mock::linux(), "\t\tjo");
        assert!(matches!(app.dialog, Some(Dialog::Route { .. })));
        // Dialog is 12 rows high in a 22 row screen; its first choice is row 8.
        click(&mut app, 30, 8);
        assert!(matches!(
            app.dialog,
            Some(Dialog::Route { selected: 0, .. })
        ));
        click(&mut app, 30, 8);
        assert!(app.dialog.is_none());
        assert_eq!(app.snapshot.streams[1].device.as_deref(), Some("builtin"));
    }

    #[test]
    fn animates_only_while_playing() {
        let mut app = app(mock::linux(), "");
        assert!(app.animating());
        app.snapshot
            .outputs
            .iter_mut()
            .for_each(|d| d.playing = false);
        app.snapshot
            .streams
            .iter_mut()
            .for_each(|s| s.playing = false);
        assert!(!app.animating());
    }

    #[test]
    fn equalizer_moves() {
        let text = |frame| {
            equalizer(frame)
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_ne!(text(0), text(3));
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
