//! Drawing only: a pure function of the app state.

use crate::app::{App, Click, Dialog, Row};
use crate::model::{Battery, ChargeState, Health, KeepAwake, PowerMode, duration};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const LABEL_WIDTH: usize = 22;
const NAME_WIDTH: usize = 24;
const BAR_WIDTH: usize = 20;
/// Columns taken by the percentage digits before the text beside them.
const DIGITS_WIDTH: usize = 16;
const USER_ROWS: u16 = 5;

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Power", header_status(app));
    if app.loaded {
        draw_body(app, frame, screen.body);
    }
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bar = key_bar(app);
    let areas = widgets::keys(frame, screen.keys, &bar);
    add_key_hits(app, &areas, &bar);
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, frame, dialog);
    }
}

fn header_status(app: &App) -> Line<'static> {
    if !app.loaded {
        return Line::raw("");
    }
    match &app.snapshot.battery {
        Some(battery) => Line::from(vec![
            widgets::battery(battery.percent),
            Span::styled(format!(" · {}", state_word(battery.state)), theme::dim()),
        ]),
        None => Line::styled("on AC power", theme::dim()),
    }
}

fn state_word(state: ChargeState) -> &'static str {
    match state {
        ChargeState::Charging => "charging",
        ChargeState::OnBattery => "on battery",
        ChargeState::Full => "full",
        ChargeState::NotCharging => "not charging",
    }
}

fn key_code(key: &str) -> Option<KeyCode> {
    match key {
        "↵" => Some(KeyCode::Enter),
        "esc" => Some(KeyCode::Esc),
        _ => key.chars().next().map(KeyCode::Char),
    }
}

fn add_key_hits(app: &App, areas: &[Rect], bindings: &[(&str, &str)]) {
    for (area, (key, _)) in areas.iter().zip(bindings) {
        if let Some(code) = key_code(key) {
            app.hits.add(*area, Click::Key(code));
        }
    }
}

/// Hits for the hint line drawn `row` lines down in a dialog.
fn add_hint_hits(app: &App, inner: Rect, row: u16, bindings: &[(&str, &str)]) {
    // Dialog hints are padded one column further than `hint` itself.
    let line = Rect {
        x: inner.x + 1,
        y: inner.y + row,
        width: inner.width.saturating_sub(1),
        height: 1,
    };
    add_key_hits(app, &widgets::hint_areas(line, bindings), bindings);
}

/// Like `widgets::dialog`, and a click outside it closes it.
fn open_dialog(app: &App, frame: &mut Frame, title: &str, width: u16, height: u16) -> Rect {
    let inner = widgets::dialog(frame, title, width, height);
    app.hits.add(frame.area(), Click::Outside);
    app.hits.add(inner.outer(Margin::new(1, 1)), Click::Inside);
    inner
}

/// Dialog hints sit two columns in, like the body text.
fn padded_hint(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut line = widgets::hint(bindings);
    line.spans.insert(0, Span::raw(" "));
    line
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    // Behind a dialog the bar is dimmed and inert, so it stays generic.
    let toggles = app.dialog.is_none()
        && app.selected_row() == Row::Mode
        && app.snapshot.mode.as_ref().is_some_and(|m| m.is_toggle());
    let action = if toggles { "toggle" } else { "change" };
    let mut keys = vec![("↵", action)];
    if app.snapshot.battery.is_some() {
        keys.push(("i", "details"));
    }
    keys.extend([("?", "more"), ("esc", "close")]);
    keys
}

fn draw_body(app: &App, frame: &mut Frame, area: Rect) {
    let mut heights = Vec::new();
    if app.snapshot.battery.is_some() {
        heights.push(Constraint::Length(7));
    }
    heights.push(Constraint::Length(app.rows().len() as u16 + 2));
    if app.snapshot.lists_energy_users {
        heights.push(Constraint::Length(USER_ROWS + 2));
    }
    let areas = Layout::vertical(heights).split(area);
    let mut areas = areas.iter();
    if let Some(battery) = &app.snapshot.battery
        && let Some(area) = areas.next()
    {
        draw_battery(frame, *area, battery);
    }
    if let Some(area) = areas.next() {
        draw_settings(app, frame, *area);
    }
    if let Some(area) = areas.next() {
        draw_users(app, frame, *area);
    }
}

fn draw_battery(frame: &mut Frame, area: Rect, battery: &Battery) {
    let block = widgets::pane("Battery", false, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let style = widgets::battery(battery.percent).style.bold();
    let digits = widgets::big_digits(&battery.percent.to_string(), style);
    let beside = [
        state_line(battery),
        Line::styled(source_text(battery), theme::dim()),
        Line::raw(""),
        Line::raw(""),
        Line::raw(""),
    ];
    let lines = digits
        .into_iter()
        .zip(beside)
        .enumerate()
        .map(|(i, (digits, text))| {
            let mut spans = vec![Span::raw("   ")];
            let used: usize = digits.spans.iter().map(Span::width).sum();
            spans.extend(digits.spans);
            let unit = if i == 4 { " %" } else { "  " };
            spans.push(Span::styled(unit, theme::dim()));
            let pad = DIGITS_WIDTH.saturating_sub(used + 2);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.extend(text.spans);
            Line::from(spans)
        });
    widgets::text(frame, inner, lines.collect());
}

/// "charging · 1h 05m until full", "on battery · 4h 12m left", "full".
fn state_line(battery: &Battery) -> Line<'static> {
    let mut spans = vec![Span::styled(state_word(battery.state), theme::text())];
    let suffix = match (battery.state, battery.minutes) {
        (ChargeState::Charging, Some(m)) => Some(format!("{} until full", duration(m))),
        (ChargeState::OnBattery, Some(m)) => Some(format!("{} left", duration(m))),
        _ => None,
    };
    if let Some(suffix) = suffix {
        spans.push(Span::styled(format!(" · {suffix}"), theme::dim()));
    }
    Line::from(spans)
}

fn source_text(battery: &Battery) -> String {
    match &battery.source {
        Some(source) => source.clone(),
        None => "not plugged in".into(),
    }
}

fn draw_settings(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::pane("Settings", true, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = app
        .rows()
        .into_iter()
        .map(|row| row_line(app, row))
        .collect();
    let selected = app.selected.min(app.rows().len() - 1);
    for (index, rect) in widgets::rows(frame, inner, lines, Some(selected)) {
        app.hits.add(rect, Click::Row(index));
    }
}

fn row_line(app: &App, row: Row) -> Line<'static> {
    let (label, target) = match row {
        Row::Mode if app.snapshot.mode.as_ref().is_some_and(|m| m.is_toggle()) => {
            ("Low Power Mode", crate::backend::MODE)
        }
        Row::Mode => ("Power mode", crate::backend::MODE),
        Row::Awake => ("Keep awake", crate::backend::AWAKE),
    };
    let mut spans = vec![Span::styled(
        widgets::fit(label, LABEL_WIDTH),
        theme::text(),
    )];
    if let Some(pending) = app.pending.get(target) {
        let text = format!("{} {pending}", widgets::spinner(app.tick));
        spans.push(Span::styled(text, theme::dim()));
    } else {
        spans.push(row_value(app, row));
    }
    Line::from(spans)
}

fn row_value(app: &App, row: Row) -> Span<'static> {
    match row {
        Row::Mode => match &app.snapshot.mode {
            Some(m) if m.is_toggle() && m.current == PowerMode::Saver => {
                Span::styled("on", theme::ok())
            }
            Some(m) if m.is_toggle() => Span::styled("off", theme::dim()),
            Some(m) => Span::styled(m.current.label(), theme::text()),
            None => Span::raw(""),
        },
        Row::Awake => match app.snapshot.keep_awake {
            KeepAwake::Off => Span::styled("off", theme::dim()),
            KeepAwake::Timed { minutes_left } => {
                Span::styled(format!("{} left", duration(minutes_left)), theme::ok())
            }
            KeepAwake::Indefinite => Span::styled("until turned off", theme::ok()),
        },
    }
}

fn draw_users(app: &App, frame: &mut Frame, area: Rect) {
    let right = app.loading_users().then(|| {
        Line::from(vec![
            Span::styled(widgets::spinner(app.tick), theme::info()),
            Span::styled(" measuring", theme::dim()),
        ])
    });
    let block = widgets::pane("Using the most energy", false, None, right);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(users) = &app.users else { return };
    if users.is_empty() {
        let line = Line::styled("   nothing is using much energy", theme::dim());
        return widgets::text(frame, inner, vec![line]);
    }
    let top = users
        .iter()
        .map(|u| u.power)
        .fold(f32::MIN_POSITIVE, f32::max);
    let lines = users
        .iter()
        .take(USER_ROWS as usize)
        .map(|user| {
            let mut spans = vec![
                Span::raw("   "),
                Span::styled(widgets::fit(&user.name, NAME_WIDTH), theme::text()),
            ];
            spans.extend(widgets::gauge(user.power / top, BAR_WIDTH, theme::info()));
            Line::from(spans)
        })
        .collect();
    widgets::text(frame, inner, lines);
}

fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    match dialog {
        Dialog::Help => draw_help(app, frame),
        Dialog::Details => draw_details(app, frame),
        Dialog::Choose { row, selected } => draw_chooser(app, frame, *row, *selected),
    }
}

fn indent(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw("  "));
    line
}

fn draw_help(app: &App, frame: &mut Frame) {
    let keys = [
        ("↵", "change the selected setting"),
        ("j k ↑ ↓", "move between settings"),
        ("i", "battery health"),
        ("esc q", "close, then quit"),
    ];
    let inner = open_dialog(app, frame, "Keys", 56, keys.len() as u16 + 4);
    let mut lines = vec![Line::raw("")];
    for (key, what) in keys {
        lines.push(Line::from(vec![
            Span::styled(widgets::fit(key, 11), theme::accent()),
            Span::styled(what, theme::text()),
        ]));
    }
    let lines = lines.into_iter().map(indent).collect();
    widgets::text(frame, inner, lines);
}

fn draw_details(app: &App, frame: &mut Frame) {
    let Some(battery) = &app.snapshot.battery else {
        return;
    };
    let health = battery.health.clone().unwrap_or(Health {
        max_capacity: None,
        cycles: None,
        condition: None,
    });
    let value = |text: Option<String>| match text {
        Some(text) => Span::raw(text),
        None => Span::styled("not reported", theme::dim()),
    };
    let rows = [
        (
            "max capacity",
            value(health.max_capacity.map(|c| format!("{c}%"))),
        ),
        ("cycles", value(health.cycles.map(|c| c.to_string()))),
        ("condition", value(health.condition)),
    ];
    let inner = open_dialog(app, frame, "Battery health", 44, rows.len() as u16 + 5);
    let mut lines = vec![Line::raw("")];
    for (label, value) in rows {
        let label = Span::styled(format!("   {label:<14}"), theme::dim());
        lines.push(Line::from(vec![label, value]));
    }
    let bindings = [("esc", "back")];
    add_hint_hits(app, inner, lines.len() as u16 + 1, &bindings);
    lines.extend([Line::raw(""), padded_hint(&bindings)]);
    widgets::text(frame, inner, lines);
}

fn draw_chooser(app: &App, frame: &mut Frame, row: Row, selected: usize) {
    let (labels, current) = app.options(row);
    let title = match row {
        Row::Mode => "Power mode",
        Row::Awake => "Keep awake",
    };
    let count = labels.len() as u16;
    let inner = open_dialog(app, frame, title, 40, count + 5);
    let lines = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let mark = if i == current { "● " } else { "  " };
            Line::from(vec![
                Span::styled(mark, theme::ok()),
                Span::styled(*label, theme::text()),
            ])
        })
        .collect();
    let list = Rect {
        y: inner.y + 1,
        height: count,
        ..inner
    };
    for (index, rect) in widgets::rows(frame, list, lines, Some(selected)) {
        app.hits.add(rect, Click::Option(index));
    }
    let bindings = [("↵", "choose"), ("esc", "cancel")];
    add_hint_hits(app, inner, count + 2, &bindings);
    let hint = Rect {
        y: inner.y + count + 2,
        height: 1,
        ..inner
    };
    widgets::text(frame, hint, vec![padded_hint(&bindings)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Cmd, Event, mock};
    use crate::model::{AwakeChoice, ModeSetting, Snapshot};
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use telmo_kit::App as _;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    struct Harness {
        app: App,
        cmds: UnboundedReceiver<Cmd>,
    }

    impl Harness {
        fn new(snapshot: Snapshot) -> Self {
            let (tx, rx) = unbounded_channel();
            let mut app = App::new(tx, Some(snapshot));
            app.event(Event::EnergyUsers(mock::energy_users()));
            Self { app, cmds: rx }
        }

        fn press(&mut self, code: KeyCode) {
            self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }

        fn chars(&mut self, keys: &str) {
            for c in keys.chars() {
                self.press(KeyCode::Char(c));
            }
        }

        fn click(&mut self, column: u16, row: u16) {
            self.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        }

        fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
            let event = MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            };
            self.app.mouse(event);
        }

        /// Draw, then find the first cell of `text` on screen.
        fn find(&self, text: &str) -> (u16, u16) {
            let screen = self.render();
            for (y, line) in screen.lines().enumerate() {
                if let Some(byte) = line.find(text) {
                    return (line[..byte].chars().count() as u16, y as u16);
                }
            }
            panic!("{text:?} not on screen:\n{screen}");
        }

        fn click_on(&mut self, text: &str) {
            let (x, y) = self.find(text);
            self.click(x, y);
        }

        fn render(&self) -> String {
            telmo_kit::test::render(90, 22, |f| self.app.draw(f))
        }

        fn sent(&mut self) -> Vec<Cmd> {
            std::iter::from_fn(|| self.cmds.try_recv().ok()).collect()
        }
    }

    fn desktop() -> Snapshot {
        Snapshot {
            battery: None,
            mode: Some(ModeSetting {
                current: PowerMode::Balanced,
                available: vec![
                    PowerMode::Saver,
                    PowerMode::Balanced,
                    PowerMode::Performance,
                ],
            }),
            keep_awake: KeepAwake::Off,
            lists_energy_users: false,
        }
    }

    fn linux_laptop() -> Snapshot {
        let mut snapshot = desktop();
        snapshot.battery = mock::snapshot().battery;
        if let Some(battery) = snapshot.battery.as_mut() {
            battery.source = Some("AC adapter".into());
        }
        snapshot
    }

    #[test]
    fn main() {
        insta::assert_snapshot!(Harness::new(mock::snapshot()).render());
    }

    #[test]
    fn low_power_on() {
        let mut snapshot = mock::snapshot();
        snapshot.mode.as_mut().unwrap().current = PowerMode::Saver;
        snapshot.keep_awake = KeepAwake::Timed { minutes_left: 47 };
        if let Some(b) = snapshot.battery.as_mut() {
            b.percent = 18;
            b.state = ChargeState::OnBattery;
            b.minutes = Some(252);
            b.source = None;
        }
        insta::assert_snapshot!(Harness::new(snapshot).render());
    }

    #[test]
    fn toggling() {
        let mut h = Harness::new(mock::snapshot());
        h.press(KeyCode::Enter);
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn awake() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("j");
        h.press(KeyCode::Enter);
        h.chars("jj");
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn details() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("i");
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn loading_users() {
        let (tx, _rx) = unbounded_channel();
        let app = App::new(tx, Some(mock::snapshot()));
        insta::assert_snapshot!(telmo_kit::test::render(90, 22, |f| app.draw(f)));
    }

    #[test]
    fn desktop_without_battery() {
        insta::assert_snapshot!(Harness::new(desktop()).render());
    }

    #[test]
    fn linux_modes() {
        let mut h = Harness::new(linux_laptop());
        h.press(KeyCode::Enter);
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn help() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("?");
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn desktop_header_and_keys() {
        let screen = Harness::new(desktop()).render();
        assert!(screen.contains("on AC power"));
        assert!(!screen.contains("i details"));
        assert!(!screen.contains("Battery"));
    }

    #[test]
    fn enter_toggles_low_power_mode() {
        let mut h = Harness::new(mock::snapshot());
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.sent().as_slice(),
            [Cmd::SetMode(PowerMode::Saver)]
        ));
        assert!(h.app.pending.contains_key(crate::backend::MODE));
        h.press(KeyCode::Enter);
        assert!(h.sent().is_empty(), "no second request while pending");
    }

    #[test]
    fn chooser_applies_selection() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("j");
        h.press(KeyCode::Enter);
        h.chars("jj");
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.sent().as_slice(),
            [Cmd::SetKeepAwake(AwakeChoice::Minutes(60))]
        ));
        assert!(h.app.dialog.is_none());
    }

    #[test]
    fn esc_closes_dialog_then_quits() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("j");
        h.press(KeyCode::Enter);
        h.press(KeyCode::Esc);
        assert!(h.app.dialog.is_none());
        assert!(h.sent().is_empty());
        assert_eq!(
            h.app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            telmo_kit::Flow::Quit
        );
    }

    #[test]
    fn click_selects_then_activates() {
        let mut h = Harness::new(mock::snapshot());
        h.click_on("Keep awake");
        assert_eq!(h.app.selected, 1);
        assert!(h.app.dialog.is_none());
        h.click_on("Keep awake");
        assert!(matches!(h.app.dialog, Some(Dialog::Choose { .. })));
    }

    #[test]
    fn click_option_applies_it() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("j");
        h.press(KeyCode::Enter);
        h.click_on("2 hours");
        assert!(matches!(
            h.sent().as_slice(),
            [Cmd::SetKeepAwake(AwakeChoice::Minutes(120))]
        ));
    }

    #[test]
    fn click_key_bar_and_dialog_hint() {
        let mut h = Harness::new(mock::snapshot());
        h.click_on("i details");
        assert!(matches!(h.app.dialog, Some(Dialog::Details)));
        h.click_on("esc back");
        assert!(h.app.dialog.is_none());
    }

    #[test]
    fn click_outside_closes_dialog() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("i");
        h.render();
        h.click(0, 0);
        assert!(h.app.dialog.is_none());
        h.chars("i");
        let (x, y) = h.find("max capacity");
        h.click(x, y);
        assert!(h.app.dialog.is_some());
    }

    #[test]
    fn scroll_moves_selection() {
        let mut h = Harness::new(mock::snapshot());
        h.render();
        h.mouse(MouseEventKind::ScrollDown, 5, 5);
        assert_eq!(h.app.selected, 1);
        h.mouse(MouseEventKind::ScrollUp, 5, 5);
        assert_eq!(h.app.selected, 0);
    }
}
