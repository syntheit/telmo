//! Drawing only: a pure function of the app state.

mod equalizer;
mod song;

use crate::app::{App, Click, Dialog, Pane};
use crate::model::Device;
use crate::rainbow;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
};
use telmo_kit::{theme, widgets};

const NAME_WIDTH: usize = 25;
const GAUGE_WIDTH: usize = 30;
/// Columns before the gauge: selection marker, dot, name and a space.
const GAUGE_X: u16 = (3 + 2 + NAME_WIDTH + 1) as u16;
/// The visualizer takes whatever the panes leave, up to this many rows.
const VIZ_ROWS: u16 = 5;

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
    if let Some(view) = &app.eq_view {
        return equalizer::draw(app, frame, view);
    }
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Sound", Line::default());
    draw_panes(app, frame, screen.body);
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bindings = fit(key_bar(app), screen.keys.width);
    let areas = widgets::keys(frame, screen.keys, &bindings);
    key_hits(app, areas, &bindings, false);
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, dialog, frame);
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    let actions = match app.pane {
        Pane::Playing => vec![("o", "output")],
        Pane::Output if app.has_eq() => vec![("e", "next preset"), ("E", "EQ")],
        _ => vec![("↵", "set default")],
    };
    let mut bindings = vec![("←→", "volume"), ("m", "mute")];
    bindings.extend(actions);
    bindings.extend([("f", "song"), ("?", "more"), ("esc", "close")]);
    bindings
}

/// Drops the song and mute keys when the bar is wider than the screen.
fn fit(
    mut bindings: Vec<(&'static str, &'static str)>,
    width: u16,
) -> Vec<(&'static str, &'static str)> {
    let used = |bindings: &[(&str, &str)]| {
        let text: usize = bindings
            .iter()
            .map(|(key, label)| key.chars().count() + 1 + label.chars().count())
            .sum();
        1 + text + 2 * bindings.len().saturating_sub(1)
    };
    for dropped in ["f", "m"] {
        if used(&bindings) > width as usize {
            bindings.retain(|(key, _)| *key != dropped);
        }
    }
    bindings
}

fn draw_panes(app: &App, frame: &mut Frame, body: Rect) {
    let panes = app.panes();
    let heights: Vec<u16> = panes
        .iter()
        .map(|p| row_count(app, *p) as u16 + 2)
        .collect();
    let spare = body.height.saturating_sub(heights.iter().sum());
    let viz = if spare >= 2 { spare.min(VIZ_ROWS) } else { 0 };
    let constraints = heights.iter().map(|h| Constraint::Length(*h));
    let areas = Layout::vertical(constraints.chain([Constraint::Fill(1), Constraint::Length(viz)]))
        .split(body);
    for (pane, area) in panes.into_iter().zip(areas.iter()) {
        draw_pane(app, frame, *area, pane);
    }
    if viz > 0 {
        draw_visualizer(app, frame, areas[areas.len() - 1]);
    }
}

const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const BLOCKED_NOTE: &str =
    "Allow System Audio Recording for Telmo in System Settings to see the visualizer.";

/// Bars one cell wide with one cell between, as many as fit, stacked eighth
/// blocks for height, a drifting rainbow across and a peak cap on each.
fn draw_visualizer(app: &App, frame: &mut Frame, area: Rect) {
    let width = area.width as usize;
    let count = width.div_ceil(2);
    if count < 2 {
        return;
    }
    let levels = resample(&app.motion.bars, count);
    let peaks = resample(&app.motion.peaks, count);
    let indent = (width - (count * 2 - 1)) / 2;
    let rows = area.height as usize;
    let lines: Vec<Line> = (0..rows)
        .rev()
        .map(|row| {
            let mut spans = vec![Span::raw(" ".repeat(indent))];
            for (i, level) in levels.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let across = i as f32 / (count - 1) as f32;
                spans.push(cell(*level, peaks[i], row, rows, across, app.motion.phase));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Text::from(lines), area);
    if app.blocked && !app.motion.lit() && rows >= 2 {
        let note = Rect {
            y: area.y + area.height - 2,
            height: 1,
            ..area
        };
        let line = Line::styled(BLOCKED_NOTE, theme::dim()).centered();
        frame.render_widget(line, note);
    }
}

/// One cell of a bar, counting rows from the bottom. The bottom row always
/// shows at least a faint baseline. A peak cap floats above a bar that has
/// fallen away from it.
fn cell(level: f32, peak: f32, row: usize, rows: usize, across: f32, phase: f32) -> Span<'static> {
    let eighths = |v: f32| (v * (rows * 8) as f32).round() as usize;
    let fill = eighths(level).saturating_sub(row * 8).min(8);
    let height = row as f32 / (rows - 1).max(1) as f32;
    let top = fill > 0 && eighths(level) <= (row + 1) * 8;
    if fill > 0 {
        let color = rainbow::bar_color(across, phase, height, top);
        return Span::styled(BLOCKS[fill - 1].to_string(), Style::new().fg(color));
    }
    let cap_row = eighths(peak).saturating_sub(1) / 8;
    if eighths(peak) > 0 && cap_row == row {
        return Span::styled("▔", Style::new().fg(rainbow::cap_color(across, phase)));
    }
    match row {
        0 => Span::styled("▁", theme::faint()),
        _ => Span::raw(" "),
    }
}

/// `count` heights spread over `bars`, linearly interpolated.
fn resample(bars: &[f32], count: usize) -> Vec<f32> {
    let last = bars.len() - 1;
    (0..count)
        .map(|i| {
            let at = i as f32 * last as f32 / (count - 1) as f32;
            let lo = (at.floor() as usize).min(last);
            let hi = (lo + 1).min(last);
            let t = at - lo as f32;
            bars[lo] * (1.0 - t) + bars[hi] * t
        })
        .collect()
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
    let tag = (pane == Pane::Output && app.has_eq()).then(|| Line::styled("EQ", eq_style(app)));
    let block = widgets::pane(title, active, None, tag);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(pane));

    let lines = match pane {
        Pane::Playing => stream_lines(app),
        _ => app
            .devices(pane)
            .iter()
            .map(|d| device_line(app, d, inner.width))
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

fn device_line(app: &App, device: &Device, width: u16) -> Line<'static> {
    let dot = if device.default {
        Span::styled("● ", theme::accent())
    } else {
        Span::raw("  ")
    };
    let mut line = level_line(dot, &device.name, device.volume, device.muted, None);
    if let Some(choice) = app.eq_choice(device) {
        let name = choice.preset().name;
        // Right-aligned, two columns from the border; the row's marker takes three.
        let room = (width as usize).saturating_sub(3 + line.width() + name.chars().count() + 2);
        if room > 0 {
            line.spans.push(Span::raw(" ".repeat(room)));
            line.spans.push(Span::styled(name, eq_style(app)));
        }
    }
    line
}

/// Magenta while the EQ is on, dim while it is off.
fn eq_style(app: &App) -> Style {
    if app.eq.enabled {
        Style::new().fg(theme::MAGENTA)
    } else {
        theme::dim()
    }
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
        Dialog::Song(listen) => song::draw(app, frame, listen),
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
    const KEYS: [(&str, &str); 13] = [
        ("tab", "switch pane"),
        ("j k", "move up and down"),
        ("h l", "volume down and up by 5%"),
        ("m", "mute or unmute"),
        ("↵", "make the device the default"),
        ("e", "next EQ preset for the output"),
        ("E", "EQ: presets, bass nudge, on/off"),
        ("o", "move the app to another output"),
        ("P", "Bluetooth mode (best sound or headset)"),
        ("f", "identify the song that's playing"),
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
    use crate::backend::{Event, mock};
    use crate::capture::mock::canned_at;
    use crate::model::Snapshot;
    use crate::spectrum::BARS;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use std::{cell::RefCell, rc::Rc};
    use telmo_kit::{App as _, runtime::KeyEvent};
    use telmo_kit::{
        Flow,
        runtime::{MouseButton, MouseEvent, MouseEventKind},
    };

    fn app(snapshot: Snapshot, keys: &str) -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (events, _events_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            Some(snapshot),
            tx,
            Rc::new(RefCell::new(None)),
            events,
            true,
        );
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

    fn canned() -> Vec<f32> {
        canned_at(0.0)
    }

    fn stopped(mut app: App) -> App {
        app.snapshot
            .outputs
            .iter_mut()
            .for_each(|d| d.playing = false);
        app.snapshot
            .streams
            .iter_mut()
            .for_each(|s| s.playing = false);
        app
    }

    /// Runs the visualizer for `frames` frames at 30 a second.
    fn run(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.motion.step(1.0);
        }
    }

    #[test]
    fn animates_only_while_audio_flows_or_fades() {
        let mut app = stopped(app(mock::linux(), ""));
        assert!(!app.animating());
        app.event(Event::Spectrum(canned()));
        assert!(app.animating());
        for _ in 0..400 {
            app.tick();
            run(&mut app, 1);
        }
        assert!(!app.animating());
        assert!(!app.motion.lit());
    }

    #[test]
    fn bars_rise_fast_and_fall_slowly() {
        let mut app = app(mock::linux(), "");
        app.motion.set_target(&vec![1.0; BARS]);
        run(&mut app, 3);
        let risen = app.motion.bars[10];
        assert!(risen > 0.7);
        app.motion.set_target(&vec![0.0; BARS]);
        run(&mut app, 3);
        assert!(app.motion.bars[10] > 0.7 * risen);
    }

    #[test]
    fn a_jump_in_the_spectrum_lands_within_a_few_frames() {
        let mut app = app(mock::linux(), "");
        app.motion.set_target(&vec![1.0; BARS]);
        run(&mut app, 1);
        assert!(app.motion.bars[10] < 1.0);
        run(&mut app, 2);
        assert!(app.motion.bars[10] > 0.95);
    }

    #[test]
    fn blocked_clears_when_sound_arrives() {
        let mut app = app(mock::mac(), "");
        app.event(Event::VisualizerBlocked);
        assert!(app.blocked);
        app.event(Event::Spectrum(canned()));
        assert!(!app.blocked);
    }

    #[test]
    fn playing_listens_and_stopping_lets_go() {
        let mut live = app(mock::linux(), "");
        live.event(Event::Snapshot(mock::linux()));
        assert!(live.capture_running());
        let quiet = stopped(app(mock::linux(), "")).snapshot;
        live.event(Event::Snapshot(quiet));
        let app = live;
        assert!(!app.capture_running());
    }

    #[test]
    fn denied_capture_is_not_retried() {
        let mut app = app(mock::mac(), "");
        app.event(Event::VisualizerBlocked);
        app.event(Event::Snapshot(mock::mac()));
        assert!(!app.capture_running());
    }

    #[test]
    fn visualizer_fills_the_width() {
        let mut app = app(mock::linux(), "");
        app.motion.show(&canned());
        let screen = render(&app);
        let row = screen.lines().nth(19).unwrap_or_default();
        assert!(
            row.chars().filter(|c| BLOCKS.contains(c)).count() >= 40,
            "{row}"
        );
    }

    #[test]
    fn resample_keeps_the_ends() {
        let bars = [0.0, 0.5, 1.0];
        assert_eq!(resample(&bars, 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    /// Caps a little above some bars, with a fixed rainbow offset.
    fn raise_caps(app: &mut App) {
        for (i, peak) in app.motion.peaks.iter_mut().enumerate() {
            *peak = (*peak + 0.12 * (i % 3) as f32).min(1.0);
        }
        app.motion.phase = 40.0;
    }

    #[test]
    fn caps_float_above_fallen_bars() {
        let mut app = app(mock::linux(), "j");
        app.motion.show(&canned());
        raise_caps(&mut app);
        assert!(render(&app).contains('▔'));
    }

    #[test]
    fn visualizer_playing_linux() {
        let mut app = app(mock::linux(), "j");
        app.motion.show(&canned());
        raise_caps(&mut app);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn visualizer_playing_mac() {
        let mut app = app(mock::mac(), "j");
        app.motion.show(&canned());
        raise_caps(&mut app);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn visualizer_blocked_mac() {
        let mut app = app(mock::mac(), "j");
        app.blocked = true;
        insta::assert_snapshot!(render(&app));
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

    // The song dialog.

    use crate::identify;
    use crate::model::Source;
    use crate::song::Phase;
    use tokio::sync::mpsc::UnboundedReceiver;

    const RATE: u32 = 16_000;

    fn with_events(snapshot: Snapshot) -> (App, UnboundedReceiver<Event>) {
        let (cmds, _cmds_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events, rx) = tokio::sync::mpsc::unbounded_channel();
        let app = App::new(
            Some(snapshot),
            cmds,
            Rc::new(RefCell::new(None)),
            events,
            true,
        );
        (app, rx)
    }

    /// `seconds` of a steady tone, as the capture would deliver it.
    fn hear(app: &mut App, source: Source, seconds: usize) {
        for _ in 0..seconds {
            let mono = vec![0.1; RATE as usize];
            app.event(Event::Samples {
                source,
                rate: RATE,
                mono,
            });
        }
    }

    /// Hands the app the answers the recognizer queued.
    fn deliver(app: &mut App, rx: &mut UnboundedReceiver<Event>) {
        while let Ok(event) = rx.try_recv() {
            if matches!(event, Event::Recognized { .. }) {
                app.event(event);
            }
        }
    }

    fn phase(app: &App) -> Phase {
        app.listen()
            .map(|l| l.phase.clone())
            .unwrap_or(Phase::NoMatch)
    }

    fn no_match(_samples: Vec<f32>, run: u64, events: crate::backend::Tx) {
        let _ = events.send(Event::Recognized {
            run,
            result: Ok(None),
        });
    }

    #[test]
    fn f_listens_to_the_desktop_while_something_plays() {
        let (mut app, _rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.listen().map(|l| l.source), Some(Source::Desktop));
        assert!(!app.mic_running());
    }

    #[test]
    fn f_listens_to_the_microphone_when_nothing_plays() {
        let (app, _rx) = with_events(stopped(app(mock::linux(), "")).snapshot);
        let mut app = app;
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.listen().map(|l| l.source), Some(Source::Mic));
        assert!(app.mic_running());
    }

    #[test]
    fn m_switches_between_microphone_and_desktop_and_starts_over() {
        let (mut app, _rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 3);
        app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        assert_eq!(app.listen().map(|l| l.source), Some(Source::Mic));
        assert!(app.mic_running());
        assert_eq!(app.listen().map(|l| l.heard_seconds()), Some(0.0));
        // Desktop audio no longer counts.
        hear(&mut app, Source::Desktop, 3);
        assert_eq!(app.listen().map(|l| l.heard_seconds()), Some(0.0));
        app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        assert_eq!(app.listen().map(|l| l.source), Some(Source::Desktop));
        assert!(!app.mic_running());
    }

    #[test]
    fn twelve_seconds_later_the_song_is_looked_up_and_remembered() {
        let (mut app, mut rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        assert_eq!(phase(&app), Phase::Recognizing);
        deliver(&mut app, &mut rx);
        assert_eq!(phase(&app), Phase::Found(identify::canned()));
        assert_eq!(app.history[0], identify::canned());
    }

    #[test]
    fn esc_cancels_listening_and_stops_the_microphone() {
        let (app, mut rx) = with_events(stopped(app(mock::linux(), "")).snapshot);
        let mut app = app;
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Mic, 12);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.dialog.is_none());
        assert!(!app.mic_running());
        // The answer that was already on its way is ignored.
        deliver(&mut app, &mut rx);
        assert!(app.dialog.is_none());
        assert_eq!(app.history.len(), 4);
    }

    #[test]
    fn no_match_listens_on_and_tries_once_more() {
        let (mut app, mut rx) = with_events(stopped(app(mock::linux(), "")).snapshot);
        app.set_recognizer(no_match);
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Mic, 12);
        deliver(&mut app, &mut rx);
        assert_eq!(phase(&app), Phase::Listening);
        assert!(app.mic_running());
        hear(&mut app, Source::Mic, 6);
        deliver(&mut app, &mut rx);
        assert_eq!(phase(&app), Phase::NoMatch);
        assert!(!app.mic_running());
        // r tries again from the start.
        app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(phase(&app), Phase::Listening);
        assert!(app.mic_running());
    }

    #[test]
    fn a_microphone_failure_shows_its_sentence() {
        let (app, _rx) = with_events(stopped(app(mock::linux(), "")).snapshot);
        let mut app = app;
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        app.event(Event::MicFailed("Turn the microphone on.".into()));
        assert_eq!(phase(&app), Phase::Failed("Turn the microphone on.".into()));
        assert!(!app.mic_running());
        assert!(app.toast.is_none());
    }

    #[test]
    fn a_desktop_failure_goes_to_the_dialog_not_a_toast() {
        let (mut app, _rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        app.event(Event::Failed("The sound server went away.".into()));
        assert_eq!(
            phase(&app),
            Phase::Failed("The sound server went away.".into())
        );
        assert!(app.toast.is_none());
    }

    #[test]
    fn keys_on_the_card_copy_and_pick_the_page() {
        let found = identify::canned();
        assert_eq!(
            crate::app::listen::page_for('o', &found),
            found.shazam_url.as_deref()
        );
        assert_eq!(
            crate::app::listen::page_for('a', &found),
            found.apple_music_url.as_deref()
        );
        assert_eq!(
            crate::app::listen::page_for('s', &found),
            Some(found.spotify_search_url.as_str())
        );
        let (mut app, mut rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let note = app.listen().and_then(|l| l.note.clone());
        assert_eq!(
            note.map(|n| n.text),
            Some("Copied to the clipboard.".into())
        );
    }

    #[test]
    fn clicking_a_hint_key_acts_like_the_key() {
        let (mut app, _rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        let screen = render(&app);
        let hint = screen
            .lines()
            .position(|l| l.contains("use microphone"))
            .unwrap_or(0);
        let line = screen.lines().nth(hint).unwrap_or_default();
        let x = line
            .find("use microphone")
            .map_or(0, |b| line[..b].chars().count())
            - 2;
        click(&mut app, x as u16, hint as u16);
        assert_eq!(app.listen().map(|l| l.source), Some(Source::Mic));
        // A click outside closes the dialog and lets go of the microphone.
        click(&mut app, 1, 1);
        assert!(app.dialog.is_none());
        assert!(!app.mic_running());
    }

    #[test]
    fn the_main_key_bar_offers_the_song_dialog() {
        let screen = render(&app(mock::linux(), ""));
        assert!(
            screen
                .lines()
                .nth(21)
                .unwrap_or_default()
                .contains("f song")
        );
    }

    #[test]
    fn song_listening_desktop_linux() {
        let (mut app, _rx) = with_events(mock::linux());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 5);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn song_listening_mic_mac() {
        let (app, _rx) = with_events(stopped(app(mock::mac(), "")).snapshot);
        let mut app = app;
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Mic, 8);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn song_result_mac() {
        let (mut app, mut rx) = with_events(mock::mac());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        insta::assert_snapshot!(render(&app));
    }

    /// A tiny two-color picture, so the half-block rendering is exact.
    fn tiny_cover() -> image::DynamicImage {
        let pixels = image::RgbaImage::from_fn(8, 8, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([200, 60, 60, 255])
            } else {
                image::Rgba([40, 40, 160, 255])
            }
        });
        image::DynamicImage::ImageRgba8(pixels)
    }

    #[test]
    fn song_result_with_cover_mac() {
        let (mut app, mut rx) = with_events(mock::mac());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        let run = app.run_id();
        app.event(Event::Cover {
            run,
            image: Some(tiny_cover()),
        });
        assert!(app.cover().is_some());
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn a_late_cover_of_an_earlier_listening_is_dropped() {
        let (mut app, mut rx) = with_events(mock::mac());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        let run = app.run_id();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.event(Event::Cover {
            run,
            image: Some(tiny_cover()),
        });
        assert!(app.cover().is_none());
    }

    /// Needs the network: `cargo test -p telmo-sound -- --ignored real_cover`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn real_cover_renders_in_half_blocks() {
        let url = "https://is1-ssl.mzstatic.com/image/thumb/Music211/v4/d6/8d/32/d68d32f6-5dec-729f-f5de-0011d0b0212e/13714.jpg/400x400bb.jpg";
        let image = crate::cover::load(url).await.expect("the cover loads");
        let (mut app, mut rx) = with_events(mock::mac());
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        let run = app.run_id();
        app.event(Event::Cover {
            run,
            image: Some(image),
        });
        assert!(app.cover().is_some());
        assert!(render(&app).contains('▀'));
    }

    #[test]
    fn song_no_match_linux() {
        let (mut app, mut rx) = with_events(mock::linux());
        app.set_recognizer(no_match);
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        hear(&mut app, Source::Desktop, 12);
        deliver(&mut app, &mut rx);
        hear(&mut app, Source::Desktop, 6);
        deliver(&mut app, &mut rx);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn song_mic_denied_mac() {
        let (app, _rx) = with_events(stopped(app(mock::mac(), "")).snapshot);
        let mut app = app;
        app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        app.event(Event::MicFailed(
            "Telmo isn't allowed to use the microphone. Turn it on in System Settings > Privacy & Security > Microphone.".into(),
        ));
        insta::assert_snapshot!(render(&app));
    }

    /// The mac mock with the visualizer showing, as in the mockups.
    fn eq_app(keys: &str) -> App {
        let mut app = app(mock::mac(), "");
        app.motion.show(&canned());
        press(&mut app, keys);
        app
    }

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            let code = match c {
                '<' => KeyCode::Left,
                '>' => KeyCode::Right,
                '^' => KeyCode::Up,
                'v' => KeyCode::Down,
                '!' => KeyCode::Esc,
                c => KeyCode::Char(c),
            };
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }
    }

    /// What the speakers have chosen, as the file would say.
    fn speakers_entry(app: &App) -> &crate::eq::Entry {
        &app.eq.devices["BuiltInSpeakerDevice#ispk"]
    }

    #[test]
    fn eq_main_shows_each_outputs_preset() {
        insta::assert_snapshot!(render(&eq_app("")));
    }

    #[test]
    fn eq_main_cycle_toast() {
        insta::assert_snapshot!(render(&eq_app("e")));
    }

    #[test]
    fn eq_main_auto_switch_toast() {
        let mut app = eq_app("je");
        app.event(Event::Snapshot(mock::mac()));
        let mut moved = mock::mac();
        moved.outputs[0].default = false;
        moved.outputs[1].default = true;
        app.event(Event::Snapshot(moved));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn eq_view_speakers() {
        insta::assert_snapshot!(render(&eq_app("E")));
    }

    #[test]
    fn eq_view_off() {
        insta::assert_snapshot!(render(&eq_app("Eb")));
    }

    #[test]
    fn eq_view_nudged() {
        insta::assert_snapshot!(render(&eq_app("E>>")));
    }

    #[test]
    fn eq_view_earfun_with_its_note() {
        insta::assert_snapshot!(render(&eq_app("jEv")));
    }

    #[test]
    fn eq_view_wired_headphones() {
        insta::assert_snapshot!(render(&eq_app("jjE")));
    }

    #[test]
    fn e_steps_through_the_presets_and_wraps() {
        let mut app = eq_app("");
        press(&mut app, "e");
        assert_eq!(speakers_entry(&app).preset, "Speakers ++");
        assert_eq!(app.eq_choice(&app.snapshot.outputs[0]).unwrap().active, 1);
        press(&mut app, "eeeee");
        assert_eq!(speakers_entry(&app).preset, "Speakers +");
        // The other outputs are untouched.
        assert!(!app.eq.devices.contains_key("EF-AA-11:output"));
    }

    #[test]
    fn e_acts_on_the_selected_output() {
        let mut app = eq_app("j");
        press(&mut app, "e");
        assert_eq!(app.eq.devices["EF-AA-11:output"].preset, "Your EarFun EQ");
        assert!(!app.eq.devices.contains_key("BuiltInSpeakerDevice#ispk"));
    }

    #[test]
    fn e_turns_the_eq_back_on() {
        let mut app = eq_app("Eb");
        press(&mut app, "!");
        assert!(!app.eq.enabled);
        press(&mut app, "e");
        assert!(app.eq.enabled);
    }

    #[test]
    fn an_output_that_cannot_be_equalized_says_so() {
        let mut app = eq_app("");
        app.snapshot.outputs[0].eq = None;
        press(&mut app, "e");
        assert!(app.toast.as_ref().is_some_and(|t| !t.ok));
        press(&mut app, "E");
        assert!(app.eq_view.is_none());
    }

    #[test]
    fn up_and_down_move_and_apply_the_preset() {
        let mut app = eq_app("E");
        press(&mut app, "v");
        assert_eq!(speakers_entry(&app).preset, "Speakers ++");
        assert_eq!(speakers_entry(&app).preamp, -4.5);
        press(&mut app, "vvvv");
        assert_eq!(speakers_entry(&app).preset, "Late night");
        press(&mut app, "v");
        assert_eq!(speakers_entry(&app).preset, "Late night");
        press(&mut app, "^");
        assert_eq!(speakers_entry(&app).preset, "Vocal");
    }

    #[test]
    fn b_switches_the_eq_off_and_on() {
        let mut app = eq_app("E");
        assert!(app.eq.enabled);
        press(&mut app, "b");
        assert!(!app.eq.enabled);
        press(&mut app, "b");
        assert!(app.eq.enabled);
    }

    #[test]
    fn the_bass_nudge_stays_within_six_and_resets() {
        let mut app = eq_app("E");
        press(&mut app, ">>>>>>>>");
        assert_eq!(speakers_entry(&app).bass, 6);
        assert_eq!(speakers_entry(&app).preamp, -9.0);
        press(&mut app, "r");
        assert_eq!(speakers_entry(&app).bass, 0);
        assert_eq!(speakers_entry(&app).preamp, -3.0);
        press(&mut app, "<<<<<<<<");
        assert_eq!(speakers_entry(&app).bass, -6);
        assert_eq!(speakers_entry(&app).preamp, -3.0);
    }

    #[test]
    fn the_nudge_is_remembered_per_output() {
        let mut app = eq_app("E");
        press(&mut app, ">>!jEv");
        assert_eq!(app.eq.devices["EF-AA-11:output"].bass, 0);
        press(&mut app, "!k");
        press(&mut app, "E");
        assert_eq!(app.eq_choice(&app.snapshot.outputs[0]).unwrap().bass, 2);
    }

    #[test]
    fn esc_goes_back_to_the_mixer() {
        let mut app = eq_app("E");
        assert!(app.eq_view.is_some());
        press(&mut app, "!");
        assert!(app.eq_view.is_none());
        assert!(!render(&app).contains("Presets"));
    }

    #[test]
    fn clicking_a_preset_applies_it() {
        let mut app = eq_app("E");
        // Header, blank, border, then the third preset.
        click(&mut app, 8, FIRST_ROW + 2);
        assert_eq!(speakers_entry(&app).preset, "Flat");
        assert_eq!(app.eq_view.as_ref().map(|v| v.cursor), Some(2));
    }

    #[test]
    fn scrolling_the_eq_view_moves_the_preset_not_the_mixer() {
        let mut app = eq_app("E");
        mouse(&mut app, MouseEventKind::ScrollDown, 8, FIRST_ROW);
        assert_eq!(speakers_entry(&app).preset, "Speakers ++");
        assert_eq!(app.selected(Pane::Output), 0);
    }

    #[test]
    fn new_outputs_are_given_their_default_preset() {
        let mut app = eq_app("");
        app.event(Event::Snapshot(mock::mac()));
        assert_eq!(speakers_entry(&app).preset, "Speakers +");
        assert_eq!(
            app.eq.devices["BuiltInHeadphoneOutputDevice#hdpn"].preset,
            "+ Sub-bass"
        );
        assert_eq!(app.eq.devices["EF-AA-11:output"].preset, "Flat");
    }

    #[test]
    fn an_output_that_disappears_closes_its_view() {
        let mut app = eq_app("jEv");
        let mut gone = mock::mac();
        gone.outputs.remove(1);
        app.event(Event::Snapshot(gone));
        assert!(app.eq_view.is_none());
    }

    #[test]
    fn linux_has_no_eq() {
        let mut app = app(mock::linux(), "");
        let screen = render(&app);
        assert!(!screen.contains("EQ") && !screen.contains("next preset"));
        press(&mut app, "e");
        assert!(app.toast.as_ref().is_some_and(|t| !t.ok));
        assert!(app.eq.devices.is_empty());
    }

    #[test]
    fn the_key_bar_drops_keys_that_do_not_fit() {
        let full = key_bar(&eq_app(""));
        assert_eq!(fit(full.clone(), 90), full);
        let narrow = fit(full, 50);
        assert!(narrow.iter().all(|(key, _)| *key != "f" && *key != "m"));
        assert!(narrow.iter().any(|(key, _)| *key == "E"));
    }
}
