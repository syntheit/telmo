//! Drawing only: a pure function of the app state.

use crate::app::{App, Dialog, Pane};
use crate::model::{Battery, Device, Kind, PairPrompt};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const NAME_WIDTH: usize = 40;
const CODE_INDENT: usize = 15;

pub fn draw(app: &App, frame: &mut Frame) {
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Bluetooth", header_status(app));
    draw_body(app, frame, screen.body);
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    widgets::keys(frame, screen.keys, &key_bar(app));
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, frame, dialog);
    }
}

fn header_status(app: &App) -> Line<'static> {
    if let Some(label) = app.pending.get("adapter") {
        let text = format!("{} {label}", widgets::spinner(app.tick));
        return Line::styled(text, theme::dim());
    }
    if app.powered() {
        Line::styled("● on", theme::ok())
    } else {
        Line::styled("○ off", theme::dim())
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    if !app.powered() {
        return vec![("p", "power"), ("esc", "close")];
    }
    // Behind a dialog the bar is dimmed and inert, so it stays generic.
    let active = app.active_pane().filter(|_| app.dialog.is_none());
    match active {
        Some(Pane::Nearby) => vec![
            ("↵", "pair"),
            ("s", "stop scan"),
            ("tab", "pane"),
            ("esc", "close"),
        ],
        Some(Pane::Connected) => generic_keys("disconnect", app.discovering()),
        _ => generic_keys("connect", app.discovering() && active.is_some()),
    }
}

fn generic_keys(action: &'static str, scanning: bool) -> Vec<(&'static str, &'static str)> {
    let scan = if scanning { "stop scan" } else { "scan" };
    vec![
        ("↵", action),
        ("s", scan),
        ("i", "details"),
        ("?", "more"),
        ("esc", "close"),
    ]
}

fn draw_body(app: &App, frame: &mut Frame, area: Rect) {
    if !app.loaded {
        return;
    }
    if app.snapshot.adapter.is_none() {
        let lines = vec![
            Line::raw(""),
            Line::styled("  Plug in or enable a Bluetooth adapter.", theme::dim()),
        ];
        return draw_notice(frame, area, "No Bluetooth adapter", lines);
    }
    if !app.powered() {
        return draw_off(app, frame, area);
    }
    draw_panes(app, frame, area);
}

fn draw_notice(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let [area] = Layout::vertical([Constraint::Length(lines.len() as u16 + 2)]).areas(area);
    let block = widgets::pane(title, false, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    widgets::text(frame, inner, lines);
}

fn draw_off(app: &App, frame: &mut Frame, area: Rect) {
    let action = match app.pending.get("adapter") {
        Some(label) => Line::styled(
            format!("  {} {label}", widgets::spinner(app.tick)),
            theme::dim(),
        ),
        None => padded_hint(&[("p", "turn on")]),
    };
    let lines = vec![Line::raw(""), action, Line::raw("")];
    draw_notice(frame, area, "Bluetooth is off", lines);
}

fn draw_panes(app: &App, frame: &mut Frame, area: Rect) {
    let panes: Vec<Pane> = Pane::ALL
        .into_iter()
        .filter(|p| *p == Pane::Nearby && app.discovering() || !app.devices(*p).is_empty())
        .collect();
    if panes.is_empty() {
        let lines = vec![Line::styled(
            "  No paired devices. Press s to look for some.",
            theme::dim(),
        )];
        return draw_notice(frame, area, "Paired", lines);
    }
    let heights = panes
        .iter()
        .map(|p| Constraint::Length(app.devices(*p).len().max(1) as u16 + 2));
    let areas = Layout::vertical(heights).split(area);
    let active = app.active_pane();
    for (pane, area) in panes.into_iter().zip(areas.iter()) {
        draw_pane(app, frame, *area, pane, active == Some(pane));
    }
}

fn draw_pane(app: &App, frame: &mut Frame, area: Rect, pane: Pane, active: bool) {
    let right = (pane == Pane::Nearby).then(|| {
        Line::from(vec![
            Span::styled(widgets::spinner(app.tick), theme::info()),
            Span::styled(" scanning", theme::dim()),
        ])
    });
    let block = widgets::pane(pane.title(), active, None, right);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let devices = app.devices(pane);
    if devices.is_empty() {
        let line = Line::styled("   looking for devices…", theme::dim());
        return widgets::text(frame, inner, vec![line]);
    }
    let selected = app.selected_device().map(|d| d.id.as_str());
    let selected_row = devices
        .iter()
        .position(|d| active && Some(d.id.as_str()) == selected);
    let lines = devices.iter().map(|d| device_line(app, d, pane)).collect();
    widgets::rows(frame, inner, lines, selected_row);
}

fn device_line(app: &App, device: &Device, pane: Pane) -> Line<'static> {
    let name = app.device_name(&device.id);
    let mut spans = vec![Span::styled(widgets::fit(&name, NAME_WIDTH), theme::text())];
    if let Some(label) = app.pending.get(&device.id) {
        let text = format!("{} {label}", widgets::spinner(app.tick));
        spans.push(Span::styled(text, theme::dim()));
    } else if pane == Pane::Connected {
        if let Some(battery) = device.battery {
            spans.extend(list_battery(battery));
        }
    } else if pane == Pane::Nearby {
        spans.extend(widgets::signal(strength(device.rssi)));
    }
    Line::from(spans)
}

/// -90 dBm and below is nothing, -40 dBm and above is full.
fn strength(rssi: Option<i16>) -> u8 {
    let dbm = i32::from(rssi.unwrap_or(-90));
    ((dbm + 90) * 2).clamp(0, 100) as u8
}

/// "61%", or "80% · case 45%" for earbuds, using the weaker bud.
fn list_battery(battery: Battery) -> Vec<Span<'static>> {
    match battery {
        Battery::Single(percent) => vec![widgets::battery(percent)],
        Battery::Buds { left, right, case } => {
            let buds = left.into_iter().chain(right).min();
            let mut spans: Vec<Span> = buds.map(widgets::battery).into_iter().collect();
            if let Some(case) = case {
                if !spans.is_empty() {
                    spans.push(Span::styled(" · ", theme::dim()));
                }
                spans.push(Span::styled("case ", theme::dim()));
                spans.push(widgets::battery(case));
            }
            spans
        }
    }
}

/// "82%", or "left 82% · right 80% · case 45%".
fn full_battery(battery: Battery) -> Vec<Span<'static>> {
    let Battery::Buds { left, right, case } = battery else {
        return list_battery(battery);
    };
    let mut spans = Vec::new();
    for (label, value) in [("left", left), ("right", right), ("case", case)] {
        let Some(value) = value else { continue };
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme::dim()));
        }
        spans.push(Span::styled(format!("{label} "), theme::dim()));
        spans.push(widgets::battery(value));
    }
    spans
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Headphones => "headphones",
        Kind::Speaker => "speaker",
        Kind::Keyboard => "keyboard",
        Kind::Mouse => "mouse",
        Kind::Gamepad => "gamepad",
        Kind::Phone => "phone",
        Kind::Computer => "computer",
        Kind::Other => "device",
    }
}

fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    match dialog {
        Dialog::Help => draw_help(frame),
        Dialog::Details(id) => draw_details(app, frame, id),
        Dialog::Rename { id, input } => {
            let title = format!("Rename {}", app.device_name(id));
            let inner = widgets::dialog(frame, &title, 50, 7);
            let mut field = vec![Span::raw("  ")];
            field.extend(input.spans(inner.width.saturating_sub(4) as usize, true));
            let hint = padded_hint(&[("↵", "save"), ("esc", "cancel")]);
            widgets::text(
                frame,
                inner,
                vec![Line::raw(""), Line::from(field), Line::raw(""), hint],
            );
        }
        Dialog::Forget(id) => {
            let title = format!("Forget {}", app.device_name(id));
            let inner = widgets::dialog(frame, &title, 60, 7);
            let lines = vec![
                Line::raw(""),
                Line::styled(
                    "  It will need to be paired again to be used.",
                    theme::text(),
                ),
                Line::raw(""),
                padded_hint(&[("y", "forget"), ("n", "cancel")]),
            ];
            widgets::text(frame, inner, lines);
        }
        Dialog::Pairing { id, prompt, input } => draw_pairing(app, frame, id, prompt, input),
    }
}

/// Dialog hints sit two columns in, like the body text.
fn padded_hint(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut line = widgets::hint(bindings);
    line.spans.insert(0, Span::raw(" "));
    line
}

fn draw_help(frame: &mut Frame) {
    let keys = [
        ("↵", "connect, disconnect or pair the selected device"),
        ("j k ↑ ↓", "move between devices"),
        ("tab", "jump to the next list"),
        ("s", "start or stop scanning"),
        ("i", "details, trust, rename, forget"),
        ("p", "turn Bluetooth on or off"),
        ("esc q", "close, then quit"),
    ];
    let inner = widgets::dialog(frame, "Keys", 64, keys.len() as u16 + 4);
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

fn indent(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw("  "));
    line
}

fn draw_details(app: &App, frame: &mut Frame, id: &str) {
    let Some(device) = app.device(id) else { return };
    let mut rows: Vec<(&str, Vec<Span>)> = Vec::new();
    if let Some(battery) = device.battery.filter(|_| device.connected) {
        rows.push(("battery", full_battery(battery)));
    }
    rows.push(("type", vec![Span::raw(kind_name(device.kind))]));
    rows.push(("address", vec![Span::raw(device.id.clone())]));

    let inner = widgets::dialog(frame, &app.device_name(id), 56, rows.len() as u16 + 6);
    let mut lines = vec![Line::raw("")];
    for (label, value) in rows {
        let mut spans = vec![Span::styled(format!("   {label:<7}    "), theme::dim())];
        spans.extend(value);
        lines.push(Line::from(spans));
    }
    let trust = if device.trusted { "untrust" } else { "trust" };
    let hint = padded_hint(&[
        ("t", trust),
        ("r", "rename"),
        ("d", "forget"),
        ("esc", "back"),
    ]);
    lines.extend([Line::raw(""), hint]);
    widgets::text(frame, inner, lines);
}

fn draw_pairing(
    app: &App,
    frame: &mut Frame,
    id: &str,
    prompt: &PairPrompt,
    input: &telmo_kit::input::TextInput,
) {
    let name = app.device_name(id);
    let is_keyboard = app.device(id).is_some_and(|d| d.kind == Kind::Keyboard);
    let thing = if is_keyboard { "keyboard" } else { "device" };
    let inner = widgets::dialog(frame, &format!("Pair {name}"), 62, 11);
    let width = inner.width as usize;

    let (message, middle, footer) = match prompt {
        PairPrompt::DisplayPasskey(code) => (
            format!("Type this code on the {thing}, then press Enter on it."),
            code_lines(*code),
            indent(Line::from(vec![
                Span::styled(
                    widgets::fit(
                        &format!("{} waiting for the {thing}", widgets::spinner(app.tick)),
                        36,
                    ),
                    theme::dim(),
                ),
                Span::styled("esc", theme::accent()),
                Span::styled(" cancel", theme::dim()),
            ])),
        ),
        PairPrompt::Confirm(code) => (
            format!("Does {name} show this code?"),
            code_lines(*code),
            padded_hint(&[("y", "yes"), ("n", "no"), ("esc", "cancel")]),
        ),
        PairPrompt::EnterPin => (
            format!("Enter the PIN shown on {name}."),
            vec![
                Line::from(input.spans(width.saturating_sub(4), true)),
                Line::raw(""),
                Line::raw(""),
            ],
            padded_hint(&[("↵", "pair"), ("esc", "cancel")]),
        ),
    };
    let mut lines = vec![
        Line::raw(""),
        Line::styled(message, theme::text()),
        Line::raw(""),
    ];
    lines.extend(middle);
    let mut lines: Vec<Line> = lines.into_iter().map(indent).collect();
    lines.extend([Line::raw(""), footer, Line::raw("")]);
    widgets::text(frame, inner, lines);
}

fn code_lines(code: u32) -> Vec<Line<'static>> {
    let digits = widgets::big_digits(&format!("{code:06}"), theme::bold().fg(theme::CYAN));
    digits
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(" ".repeat(CODE_INDENT - 2)));
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Cmd, Event, mock};
    use crate::model::Snapshot;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use telmo_kit::App as _;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    struct Harness {
        app: App,
        _cmds: UnboundedReceiver<Cmd>,
    }

    impl Harness {
        fn new(snapshot: Snapshot) -> Self {
            let (tx, rx) = unbounded_channel();
            Self {
                app: App::new(tx, Some(snapshot)),
                _cmds: rx,
            }
        }

        fn press(&mut self, code: KeyCode) {
            self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }

        fn chars(&mut self, keys: &str) {
            for c in keys.chars() {
                self.press(KeyCode::Char(c));
            }
        }

        fn render(&self) -> String {
            telmo_kit::test::render(90, 22, |f| self.app.draw(f))
        }
    }

    fn scanning() -> Snapshot {
        let mut snapshot = mock::snapshot();
        snapshot.devices.extend(mock::nearby_devices());
        if let Some(adapter) = snapshot.adapter.as_mut() {
            adapter.discovering = true;
        }
        snapshot
    }

    fn nearby_selected() -> Harness {
        let mut h = Harness::new(scanning());
        h.press(KeyCode::Tab);
        h.press(KeyCode::Tab);
        h
    }

    #[test]
    fn devices() {
        insta::assert_snapshot!(Harness::new(mock::snapshot()).render());
    }

    #[test]
    fn connecting() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("jj");
        h.press(KeyCode::Enter);
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn scan() {
        insta::assert_snapshot!(nearby_selected().render());
    }

    #[test]
    fn pair() {
        let mut h = nearby_selected();
        h.press(KeyCode::Enter);
        h.app.event(Event::Pairing {
            device: mock::nearby_devices()[0].id.clone(),
            prompt: PairPrompt::DisplayPasskey(482913),
        });
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn details() {
        let mut h = Harness::new(mock::snapshot());
        h.chars("i");
        insta::assert_snapshot!(h.render());
    }

    #[test]
    fn off() {
        let mut snapshot = mock::snapshot();
        snapshot
            .devices
            .iter_mut()
            .for_each(|d| d.connected = false);
        if let Some(adapter) = snapshot.adapter.as_mut() {
            adapter.powered = false;
        }
        insta::assert_snapshot!(Harness::new(snapshot).render());
    }
}
