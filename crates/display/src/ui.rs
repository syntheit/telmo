//! Drawing only: a pure function of the app state.

use crate::app::{App, Click, Dialog, Pane};
use crate::model::{Display, Mode};
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const NAME_WIDTH: usize = 30;
const GAUGE_WIDTH: usize = 30;
/// Columns before the gauge: selection marker, dot, name and a space.
const GAUGE_X: u16 = (3 + 2 + NAME_WIDTH + 1) as u16;

fn key_code(key: &str) -> Option<KeyCode> {
    match key {
        "↵" => Some(KeyCode::Enter),
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        "←→" => Some(KeyCode::Right),
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
    widgets::header(frame, screen.header, "Display", Line::default());
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
    match app.pane {
        Pane::Displays => vec![
            ("←→", "brightness"),
            ("↵", "size"),
            ("i", "details"),
            ("n", "night shift"),
            ("?", "more"),
            ("esc", "close"),
        ],
        Pane::Night => vec![
            ("←→", "warmth"),
            ("↵", "on or off"),
            ("tab", "displays"),
            ("?", "more"),
            ("esc", "close"),
        ],
    }
}

fn draw_panes(app: &App, frame: &mut Frame, body: Rect) {
    let rows = app.snapshot.displays.len().max(1) as u16;
    let areas = Layout::vertical([
        Constraint::Length(rows + 2),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .split(body);
    draw_displays(app, frame, areas[0]);
    draw_night(app, frame, areas[1]);
}

fn draw_displays(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.pane == Pane::Displays;
    let block = widgets::pane("Displays", active, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Pane::Displays));
    if app.snapshot.displays.is_empty() {
        let line = Line::styled("   No displays found.", theme::dim());
        return widgets::text(frame, inner, vec![line]);
    }
    let lines = app.snapshot.displays.iter().map(display_line).collect();
    let selected = active.then_some(app.selected);
    for (i, rect) in widgets::rows(frame, inner, lines, selected) {
        app.hits.add(rect, Click::Row(Pane::Displays, i));
        if app.snapshot.displays[i].brightness.is_some() {
            gauge_hits(app, rect, Pane::Displays, i);
        }
    }
}

fn draw_night(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.pane == Pane::Night;
    let block = widgets::pane("Night Shift", active, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Pane::Night));
    let night = &app.snapshot.night;
    let mut spans = vec![
        if night.on && night.available {
            Span::styled("● ", theme::accent())
        } else {
            Span::raw("  ")
        },
        Span::styled(widgets::fit("Warmth", NAME_WIDTH), theme::text()),
        Span::raw(" "),
    ];
    if night.available {
        let style = if night.on {
            theme::accent()
        } else {
            theme::dim()
        };
        spans.extend(widgets::gauge(night.warmth, GAUGE_WIDTH, style));
        spans.push(if night.on {
            let percent = format!("{}%", (night.warmth * 100.0).round() as u32);
            Span::styled(format!("{percent:>7}"), theme::text())
        } else {
            Span::styled(format!("{:>7}", "off"), theme::dim())
        });
    } else {
        spans.push(Span::styled("unavailable", theme::faint()));
    }
    let rows = widgets::rows(frame, inner, vec![Line::from(spans)], active.then_some(0));
    for (i, rect) in rows {
        app.hits.add(rect, Click::Row(Pane::Night, i));
        if night.available {
            gauge_hits(app, rect, Pane::Night, i);
        }
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
        let level = cell as f32 / (GAUGE_WIDTH - 1) as f32;
        app.hits.add(rect, Click::Gauge(pane, i, level));
    }
}

fn display_line(display: &Display) -> Line<'static> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(widgets::fit(&display.name, NAME_WIDTH), theme::text()),
        Span::raw(" "),
    ];
    match display.brightness {
        None => spans.push(Span::styled("brightness not adjustable", theme::faint())),
        Some(level) => {
            spans.extend(widgets::gauge(level, GAUGE_WIDTH, theme::accent()));
            let percent = format!("{}%", (level * 100.0).round() as u32);
            spans.push(Span::styled(format!("{percent:>7}"), theme::text()));
        }
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
        Dialog::Size { display, selected } => draw_size(app, frame, display, *selected),
        Dialog::Details { display } => draw_details(app, frame, display),
    }
}

fn padded(hint: Line<'static>) -> Line<'static> {
    let mut hint = hint;
    hint.spans.insert(0, Span::raw(" "));
    hint
}

/// The hint line is drawn one column further in than `widgets::hint` alone.
fn hint_hits(app: &App, row: Rect, bindings: &[(&str, &str)]) {
    let row = Rect {
        x: row.x + 1,
        ..row
    };
    key_hits(app, widgets::hint_areas(row, bindings), bindings, true);
}

fn mode_line(mode: &Mode) -> Line<'static> {
    let dot = if mode.current {
        Span::styled("● ", theme::accent())
    } else {
        Span::raw("  ")
    };
    Line::from(vec![
        dot,
        Span::styled(format!("{:<14}", mode.label), theme::text()),
        Span::styled(mode.note.clone(), theme::dim()),
    ])
}

/// Body: blank, the choices, blank, an optional note and blank, the hint.
fn draw_size(app: &App, frame: &mut Frame, display: &str, selected: usize) {
    let Some(display) = app.display(display) else {
        return;
    };
    let count = display.modes.len() as u16;
    let extra = if display.modes_note.is_some() { 2 } else { 0 };
    let title = format!("{} size", display.name);
    let inner = widgets::dialog(frame, &title, 70, count + extra + 6);
    dialog_hits(app, frame, inner);
    let list = Rect {
        y: inner.y + 1,
        height: count,
        ..inner
    };
    let lines = display.modes.iter().map(mode_line).collect();
    for (i, rect) in widgets::rows(frame, list, lines, Some(selected)) {
        app.hits.add(rect, Click::DialogRow(i));
    }
    if let Some(note) = &display.modes_note {
        let row = Rect {
            y: inner.y + count + 2,
            height: 1,
            ..inner
        };
        frame.render_widget(Line::styled(format!("   {note}"), theme::dim()), row);
    }
    let bindings = [("↵", "apply"), ("esc", "cancel")];
    let hint_row = Rect {
        y: inner.y + count + extra + 2,
        height: 1,
        ..inner
    };
    frame.render_widget(padded(widgets::hint(&bindings)), hint_row);
    hint_hits(app, hint_row, &bindings);
}

fn draw_details(app: &App, frame: &mut Frame, display: &str) {
    let Some(display) = app.display(display) else {
        return;
    };
    let info = &display.info;
    let facts = [
        ("Resolution", info.resolution.clone()),
        ("Refresh rate", info.refresh.map(|hz| format!("{hz:.0} Hz"))),
        ("Scale", info.scale.map(|s| format!("{s}x"))),
        ("Connection", info.connection.clone()),
    ];
    let mut lines = vec![Line::raw("")];
    for (label, value) in facts {
        if let Some(value) = value {
            lines.push(Line::from(vec![
                Span::styled(format!("   {label:<14}"), theme::dim()),
                Span::styled(value, theme::text()),
            ]));
        }
    }
    let back = [("esc", "back")];
    let hint_row = Rect {
        height: 1,
        ..Rect::default()
    };
    lines.push(Line::raw(""));
    lines.push(padded(widgets::hint(&back)));
    let inner = widgets::dialog(frame, &display.name, 52, lines.len() as u16 + 3);
    dialog_hits(app, frame, inner);
    let hint_row = Rect {
        y: inner.y + lines.len() as u16 - 1,
        width: inner.width,
        x: inner.x,
        ..hint_row
    };
    hint_hits(app, hint_row, &back);
    widgets::text(frame, inner, lines);
}

fn draw_help(app: &App, frame: &mut Frame) {
    const KEYS: [(&str, &str); 9] = [
        ("tab", "switch between displays and Night Shift"),
        ("j k", "move up and down"),
        ("h l", "brightness or warmth down and up by 5%"),
        ("↵", "pick a size, or switch Night Shift on or off"),
        ("n", "Night Shift on or off"),
        ("i", "details: resolution, refresh rate, connection"),
        ("?", "this help"),
        ("esc", "close a dialog, or quit"),
        ("q", "quit"),
    ];
    let inner = widgets::dialog(frame, "Keys", 64, KEYS.len() as u16 + 2);
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
    use telmo_kit::runtime::{MouseButton, MouseEventKind};
    use telmo_kit::{App as _, Flow, runtime::KeyEvent};

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
        app.mouse(telmo_kit::test::mouse_event(kind, column, row))
    }

    fn click(app: &mut App, column: u16, row: u16) -> Flow {
        mouse(app, MouseEventKind::Down(MouseButton::Left), column, row)
    }

    /// Screen row of the first display (header, blank, border).
    const FIRST_ROW: u16 = 3;
    /// Screen row of the Night Shift row on the mac mock (3 displays).
    const NIGHT_ROW: u16 = 8;

    fn brightness(app: &App, i: usize) -> Option<f32> {
        app.snapshot.displays[i].brightness
    }

    #[test]
    fn keys_adjust_with_optimistic_update() {
        let mut app = app(mock::mac(), "");
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(brightness(&app, 0), Some(0.65));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(brightness(&app, 0), Some(0.55));
    }

    #[test]
    fn click_selects_then_opens_size() {
        let mut app = app(mock::mac(), "");
        click(&mut app, 10, FIRST_ROW + 1);
        assert_eq!(app.selected, 1);
        assert!(app.dialog.is_none());
        click(&mut app, 10, FIRST_ROW + 1);
        assert!(matches!(app.dialog, Some(Dialog::Size { .. })));
    }

    #[test]
    fn click_gauge_sets_brightness() {
        let mut app = app(mock::mac(), "");
        let x = 1 + GAUGE_X;
        click(&mut app, x, FIRST_ROW + 1);
        assert_eq!(app.selected, 1);
        assert_eq!(brightness(&app, 1), Some(0.0));
        click(&mut app, x + GAUGE_WIDTH as u16 - 1, FIRST_ROW + 1);
        assert_eq!(brightness(&app, 1), Some(1.0));
    }

    #[test]
    fn click_night_gauge_turns_it_on() {
        let mut app = app(mock::mac(), "");
        click(&mut app, 1 + GAUGE_X + GAUGE_WIDTH as u16 - 1, NIGHT_ROW);
        assert_eq!(app.pane, Pane::Night);
        assert!(app.snapshot.night.on);
        assert_eq!(app.snapshot.night.warmth, 1.0);
        click(&mut app, 10, NIGHT_ROW);
        assert!(!app.snapshot.night.on);
    }

    #[test]
    fn scroll_adjusts_or_moves() {
        let mut app = app(mock::mac(), "");
        mouse(&mut app, MouseEventKind::ScrollUp, 10, FIRST_ROW);
        assert_eq!(brightness(&app, 0), Some(0.65));
        mouse(&mut app, MouseEventKind::ScrollDown, 85, 1);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn click_key_bar_and_dialog() {
        let mut app = app(mock::mac(), "");
        let bar = render(&app);
        let last = bar.lines().nth(21).unwrap_or_default().to_string();
        let x = last
            .find('?')
            .map(|b| last[..b].chars().count())
            .unwrap_or(0);
        click(&mut app, x as u16, 21);
        assert!(matches!(app.dialog, Some(Dialog::Help)));
        click(&mut app, 45, 11);
        assert!(app.dialog.is_some());
        click(&mut app, 1, 1);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn click_size_row_then_confirm() {
        let mut app = app(mock::mac(), "\n");
        assert!(matches!(app.dialog, Some(Dialog::Size { selected: 2, .. })));
        // Dialog is 11 rows high; its first choice is on row 7.
        click(&mut app, 30, 7);
        assert!(matches!(app.dialog, Some(Dialog::Size { selected: 0, .. })));
        click(&mut app, 30, 7);
        assert!(app.dialog.is_none());
        assert!(app.snapshot.displays[0].modes[0].current);
    }

    #[test]
    fn keyboard_has_no_size_or_details() {
        let mut app = app(mock::mac(), "jj\n");
        assert!(app.dialog.is_none() && app.toast.is_some());
        app.toast = None;
        app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(app.dialog.is_none() && app.toast.is_some());
    }

    #[test]
    fn unadjustable_display_refuses_with_toast() {
        let mut app = app(mock::linux(), "j");
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.toast.is_some());
        assert_eq!(brightness(&app, 1), None);
    }

    #[test]
    fn mac_displays() {
        insta::assert_snapshot!(render(&app(mock::mac(), "")));
    }

    #[test]
    fn mac_keyboard() {
        insta::assert_snapshot!(render(&app(mock::mac(), "jj")));
    }

    #[test]
    fn mac_night() {
        insta::assert_snapshot!(render(&app(mock::mac(), "n\t")));
    }

    #[test]
    fn mac_size() {
        insta::assert_snapshot!(render(&app(mock::mac(), "\n")));
    }

    #[test]
    fn mac_details() {
        insta::assert_snapshot!(render(&app(mock::mac(), "ji")));
    }

    #[test]
    fn mac_help() {
        insta::assert_snapshot!(render(&app(mock::mac(), "?")));
    }

    #[test]
    fn linux_displays() {
        insta::assert_snapshot!(render(&app(mock::linux(), "j")));
    }

    #[test]
    fn linux_size() {
        insta::assert_snapshot!(render(&app(mock::linux(), "\n")));
    }

    #[test]
    fn linux_night_unavailable() {
        let mut snapshot = mock::linux();
        snapshot.night.available = false;
        insta::assert_snapshot!(render(&app(snapshot, "\t")));
    }
}
