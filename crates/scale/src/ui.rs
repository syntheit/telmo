//! Drawing only, a pure function of the app state.

use crate::app::{App, Unit};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const INSTRUCTION: &str =
    "Rest a finger on the trackpad, then set the object on it — keep touching.";

pub fn draw(app: &App, frame: &mut Frame) {
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Scale", header_status(app));
    if app.loaded {
        match &app.snapshot.unavailable {
            Some(reason) => draw_unavailable(frame, screen.body, reason),
            None => draw_readout(app, frame, screen.body),
        }
    }
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    widgets::keys(frame, screen.keys, &key_bar(app));
    if app.help {
        draw_help(frame);
    }
}

fn header_status(app: &App) -> Line<'static> {
    if !app.loaded || !app.available() {
        Line::raw("")
    } else if app.snapshot.touching {
        Line::styled("● touching", theme::ok())
    } else {
        Line::styled("○ no touch", theme::dim())
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    let unit = match app.unit {
        Unit::Grams => "ounces",
        Unit::Ounces => "grams",
    };
    if app.available() {
        vec![
            ("space", "zero"),
            ("u", unit),
            ("?", "more"),
            ("esc", "close"),
        ]
    } else {
        vec![("esc", "close")]
    }
}

fn draw_unavailable(frame: &mut Frame, area: Rect, reason: &str) {
    let [area] = Layout::vertical([Constraint::Length(5)]).areas(area);
    let block = widgets::pane("No trackpad", false, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::raw(""),
        Line::styled(format!("  {reason}"), theme::dim()),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_readout(app: &App, frame: &mut Frame, area: Rect) {
    let [area] = Layout::vertical([Constraint::Length(11)]).areas(area);
    let block = widgets::pane("Weight", false, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let touching = app.snapshot.touching;
    let style = if touching {
        theme::text().bold()
    } else {
        theme::faint()
    };
    // The unit sits beside the bottom row; pad the rows above by the same
    // width so all three stay aligned when centered.
    let unit = format!("  {}", app.unit_label());
    let mut digits = widgets::big_digits(&app.readout(), style);
    for (i, row) in digits.iter_mut().enumerate() {
        let tail = if i == 2 {
            unit.clone()
        } else {
            " ".repeat(unit.chars().count())
        };
        row.spans.push(Span::styled(tail, theme::dim()));
    }

    let mut lines = vec![Line::raw("")];
    lines.extend(digits);
    lines.push(Line::raw(""));
    lines.push(indicator(app));
    lines.push(Line::raw(""));
    lines.push(Line::styled(INSTRUCTION, theme::dim()));
    let lines = lines.into_iter().map(Line::centered).collect();
    widgets::text(frame, inner, lines);
}

fn indicator(app: &App) -> Line<'static> {
    if !app.snapshot.touching {
        Line::raw("")
    } else if app.snapshot.stable {
        Line::styled("● stable", theme::ok())
    } else {
        Line::styled("◌ settling…", theme::warn())
    }
}

fn draw_help(frame: &mut Frame) {
    let keys = [
        ("space", "zero the scale"),
        ("u", "switch between grams and ounces"),
        ("esc q", "close"),
    ];
    let inner = widgets::dialog(frame, "Keys", 50, keys.len() as u16 + 4);
    let mut lines = vec![Line::raw("")];
    for (key, what) in keys {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(widgets::fit(key, 9), theme::accent()),
            Span::styled(what, Style::new().fg(theme::FG)),
        ]));
    }
    widgets::text(frame, inner, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Event, mock};
    use crate::model::Snapshot;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use telmo_kit::App as _;

    fn app_with(snapshot: Snapshot) -> App {
        let mut app = App::new();
        app.event(Event::Snapshot(snapshot));
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn render(app: &App) -> String {
        telmo_kit::test::render(90, 22, |f| draw(app, f))
    }

    #[test]
    fn idle() {
        insta::assert_snapshot!(render(&app_with(mock::idle())));
    }

    #[test]
    fn settling() {
        insta::assert_snapshot!(render(&app_with(mock::snapshot(117.0, false))));
    }

    #[test]
    fn stable() {
        insta::assert_snapshot!(render(&app_with(mock::snapshot(124.5, true))));
    }

    #[test]
    fn ounces() {
        let mut app = app_with(mock::snapshot(124.5, true));
        press(&mut app, KeyCode::Char('u'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn tared() {
        let mut app = app_with(mock::snapshot(80.0, true));
        press(&mut app, KeyCode::Char(' '));
        app.event(Event::Snapshot(mock::snapshot(204.5, true)));
        assert_eq!(app.readout(), "124.5");
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn tare_resets_on_lift() {
        let mut app = app_with(mock::snapshot(80.0, true));
        press(&mut app, KeyCode::Char(' '));
        app.event(Event::Snapshot(mock::idle()));
        app.event(Event::Snapshot(mock::snapshot(50.0, true)));
        assert_eq!(app.readout(), "50.0");
    }

    #[test]
    fn tare_without_touch() {
        let mut app = app_with(mock::idle());
        press(&mut app, KeyCode::Char(' '));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn help() {
        let mut app = app_with(mock::snapshot(124.5, true));
        press(&mut app, KeyCode::Char('?'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn unavailable() {
        let snapshot = Snapshot {
            unavailable: Some("Needs a Mac with a Force Touch trackpad.".into()),
            ..Snapshot::default()
        };
        insta::assert_snapshot!(render(&app_with(snapshot)));
    }
}
