//! Drawing only: a pure function of the app state.

use crate::{
    app::{App, Click, Dialog, Tab},
    fmt, model,
    model::{Timer, TimerStatus},
    parse,
};
use crossterm::event::KeyCode;
use jiff::{Timestamp, tz::TimeZone};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use telmo_kit::{theme, widgets};

const GAUGE: usize = 24;

/// What a timer is called: its name, or its length.
pub fn timer_title(timer: &Timer) -> String {
    if timer.name.is_empty() {
        format!("{} timer", fmt::short(timer.duration_ms))
    } else {
        timer.name.clone()
    }
}

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    let screen = widgets::screen(frame.area());
    draw_header(app, frame, screen.header);
    match app.tab {
        Tab::Timers => draw_timers(app, frame, screen.body),
        Tab::Stopwatch => draw_stopwatch(app, frame, screen.body),
        Tab::World => draw_world(app, frame, screen.body),
        Tab::Alarms => draw_alarms(app, frame, screen.body),
    }
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bar = key_bar(app.tab);
    let areas = widgets::keys(frame, screen.keys, &bar);
    add_key_hits(app, &areas, &bar);
    match &app.dialog {
        Some(Dialog::Help) => draw_help(app, frame),
        Some(Dialog::NewTimer { input, .. }) => draw_new_timer(app, frame, input),
        Some(Dialog::NewAlarm { input }) => draw_new_alarm(app, frame, input),
        None => {}
    }
}

// --- header and key bar ---

fn draw_header(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x + 1;
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme::faint()));
            x += 3;
        }
        let style = if tab == app.tab {
            theme::accent().bold()
        } else {
            theme::dim()
        };
        let width = tab.label().chars().count() as u16;
        app.hits.add(
            Rect {
                x,
                y: area.y,
                width,
                height: 1,
            },
            Click::Tab(tab),
        );
        spans.push(Span::styled(tab.label(), style));
        x += width;
    }
    frame.render_widget(Line::from(spans), area);
    let mut status = Line::styled(status_text(app), theme::dim());
    status.spans.push(Span::raw(" "));
    frame.render_widget(status.right_aligned(), area);
}

fn status_text(app: &App) -> String {
    let now = app.now();
    match app.tab {
        Tab::Timers => {
            let (running, paused) = app.running_summary();
            let parts: Vec<String> = [(running, "running"), (paused, "paused")]
                .into_iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, word)| format!("{n} {word}"))
                .collect();
            parts.join(" · ")
        }
        Tab::Stopwatch => if app.state.stopwatch.running() {
            "running"
        } else {
            ""
        }
        .into(),
        Tab::World => zoned(now, &app.tz).strftime("%a %-d %b").to_string(),
        Tab::Alarms => app
            .state
            .alarms
            .iter()
            .filter_map(|a| a.due_ms())
            .min()
            .map(|due| format!("next {}", fmt::until(due - now)))
            .unwrap_or_default(),
    }
}

fn key_bar(tab: Tab) -> Vec<(&'static str, &'static str)> {
    let mut keys: Vec<(&'static str, &'static str)> = match tab {
        Tab::Timers => vec![
            ("n", "new"),
            ("↵", "pause"),
            ("r", "restart"),
            ("x", "delete"),
        ],
        Tab::Stopwatch => vec![("space", "start/stop"), ("l", "lap"), ("r", "reset")],
        Tab::World => vec![("↑↓", "select")],
        Tab::Alarms => vec![("n", "new"), ("↵", "on/off"), ("x", "delete")],
    };
    keys.extend([("←→", "tab"), ("?", "more"), ("esc", "close")]);
    keys
}

fn key_code(key: &str) -> Option<KeyCode> {
    match key {
        "↵" => Some(KeyCode::Enter),
        "esc" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        "←→" | "↑↓" => None,
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

// --- small helpers ---

fn zoned(ms: i64, tz: &TimeZone) -> jiff::Zoned {
    Timestamp::from_millisecond(ms)
        .unwrap_or_default()
        .to_zoned(tz.clone())
}

fn hhmm(ms: i64, tz: &TimeZone) -> String {
    zoned(ms, tz).strftime("%H:%M").to_string()
}

/// `14:12`, or `Sun 02:07` when it is not today.
fn when(ms: i64, now: i64, tz: &TimeZone) -> String {
    if zoned(ms, tz).date() == zoned(now, tz).date() {
        hhmm(ms, tz)
    } else {
        zoned(ms, tz).strftime("%a %H:%M").to_string()
    }
}

fn indent(mut line: Line<'static>, by: usize) -> Line<'static> {
    line.spans.insert(0, Span::raw(" ".repeat(by)));
    line
}

fn digits(text: &str, style: Style) -> Vec<Line<'static>> {
    widgets::big_digits(text, style)
        .into_iter()
        .map(|l| indent(l, 3))
        .collect()
}

fn empty_state(frame: &mut Frame, area: Rect, title: &str, lines: [&str; 2]) {
    let block = widgets::pane(title, true, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = lines.map(|l| Line::styled(format!("  {l}"), theme::dim()));
    widgets::text(
        frame,
        inner,
        [Line::raw("")].into_iter().chain(lines).collect(),
    );
}

/// A pane with `lines`, and the inner area it was drawn in.
fn pane(
    frame: &mut Frame,
    area: Rect,
    block: ratatui::widgets::Block,
    lines: Vec<Line<'static>>,
) -> Rect {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
    inner
}

fn status_style(status: TimerStatus) -> Style {
    match status {
        TimerStatus::Running => theme::accent(),
        TimerStatus::Paused => theme::warn(),
        TimerStatus::Done => theme::ok(),
    }
}

// --- timers ---

fn draw_timers(app: &App, frame: &mut Frame, area: Rect) {
    let now = app.now();
    let Some(selected) = app.state.timers.get(app.timer) else {
        let [_, rest] = Layout::vertical([Constraint::Length(0), Constraint::Fill(1)]).areas(area);
        empty_state(
            frame,
            rest,
            "Timers",
            [
                "No timers. Press n to start one,",
                "or run `telmo-clock timer start 5m`.",
            ],
        );
        return;
    };
    let top_height = if area.height >= 14 { 10 } else { 0 };
    let [top, list] =
        Layout::vertical([Constraint::Length(top_height), Constraint::Fill(1)]).areas(area);
    if top_height > 0 {
        let status = selected.status(now);
        let style = status_style(status);
        let big = match status {
            TimerStatus::Done => fmt::countdown(0),
            _ => fmt::countdown(selected.remaining(now)),
        };
        let mut lines = digits(
            &big,
            if status == TimerStatus::Running {
                theme::bold()
            } else {
                style
            },
        );
        lines.push(Line::raw(""));
        let width = (top.width as usize).saturating_sub(2 + 6);
        let left = 1.0 - selected.progress(now);
        lines.push(indent(Line::from(widgets::gauge(left, width, style)), 3));
        let info = match status {
            TimerStatus::Running => format!(
                "running · ends {}",
                when(selected.ends_at_ms.unwrap_or(now), now, &app.tz)
            ),
            TimerStatus::Paused => "paused".to_string(),
            TimerStatus::Done => format!(
                "done at {}",
                when(selected.ends_at_ms.unwrap_or(now), now, &app.tz)
            ),
        };
        lines.push(Line::styled(format!("   {info}"), theme::dim()));
        let title = timer_title(selected);
        let right = Line::styled(fmt::short(selected.duration_ms), theme::dim());
        pane(
            frame,
            top,
            widgets::pane(&title, false, None, Some(right)),
            lines,
        );
    }

    let rows: Vec<Line> = app
        .state
        .timers
        .iter()
        .map(|t| {
            let status = t.status(now);
            let style = status_style(status);
            let word = match status {
                TimerStatus::Running => "running",
                TimerStatus::Paused => "paused",
                TimerStatus::Done => "done",
            };
            let mut spans = vec![
                Span::styled(widgets::fit(&timer_title(t), 16), theme::text()),
                Span::styled(format!("{:>9}", fmt::countdown(t.remaining(now))), style),
                Span::raw("  "),
            ];
            spans.extend(widgets::gauge(1.0 - t.progress(now), GAUGE, style));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(word, style));
            Line::from(spans)
        })
        .collect();
    let badge = Some(rows.len().to_string());
    let block = widgets::pane("Timers", true, badge, None);
    let inner = block.inner(list);
    frame.render_widget(block, list);
    for (index, rect) in widgets::rows(frame, inner, rows, Some(app.timer)) {
        app.hits.add(rect, Click::Row(index));
    }
}

// --- stopwatch ---

fn draw_stopwatch(app: &App, frame: &mut Frame, area: Rect) {
    let now = app.now();
    let watch = &app.state.stopwatch;
    let [top, laps] = Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area);

    let elapsed = watch.elapsed(now);
    let style = if watch.running() {
        theme::bold()
    } else {
        theme::dim()
    };
    let mut lines = digits(&fmt::tenths(elapsed), style);
    lines.push(Line::raw(""));
    let info = match (watch.running(), watch.laps.is_empty()) {
        (true, true) => "press l to mark a lap".to_string(),
        (true, false) => format!(
            "lap {} · {} since the last lap",
            watch.laps.len() + 1,
            fmt::tenths(watch.current_split(now))
        ),
        (false, _) if elapsed == 0 => "press space to start".to_string(),
        (false, _) => "stopped".to_string(),
    };
    lines.push(Line::styled(format!("   {info}"), theme::dim()));
    let label = if watch.running() {
        "running"
    } else if elapsed == 0 {
        "ready"
    } else {
        "stopped"
    };
    let right = Line::styled(label, theme::dim());
    pane(
        frame,
        top,
        widgets::pane("Stopwatch", false, None, Some(right)),
        lines,
    );

    let done = watch.laps();
    let badge = (!done.is_empty()).then(|| done.len().to_string());
    let block = widgets::pane("Laps", true, badge, None);
    let mut rows = Vec::new();
    if done.is_empty() {
        rows.push(Line::styled(
            "   No laps yet. Press l while it runs.",
            theme::dim(),
        ));
    } else {
        rows.push(Line::styled(
            "    Lap      Split       Total        vs previous",
            theme::dim(),
        ));
        for lap in done.iter().rev() {
            let delta = lap.delta_ms.map_or(String::new(), fmt::delta);
            let delta_style = match lap.delta_ms {
                Some(d) if d < 0 => theme::ok(),
                Some(d) if d > 0 => theme::err(),
                _ => theme::dim(),
            };
            rows.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{:<6}", format!("#{}", lap.number)), theme::dim()),
                Span::styled(
                    format!("{:>9}", fmt::hundredths(lap.split_ms)),
                    theme::text(),
                ),
                Span::styled(
                    format!("    {:>9}", fmt::hundredths(lap.total_ms)),
                    theme::dim(),
                ),
                Span::styled(format!("    {delta:>12}"), delta_style),
            ]));
        }
    }
    pane(frame, laps, block, rows);
}

// --- world ---

fn draw_world(app: &App, frame: &mut Frame, area: Rect) {
    let now = app.now();
    if app.places.is_empty() {
        empty_state(
            frame,
            area,
            "Cities",
            ["No cities. Add some with", "programs.telmo.clock.cities."],
        );
        return;
    }
    let local = zoned(now, &app.tz);
    let [top, list] = Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area);

    let selected = &app.places[app.city.min(app.places.len() - 1)];
    match &selected.zone {
        Some(zone) => {
            let there = zoned(now, zone);
            let mut lines = digits(&there.strftime("%H:%M").to_string(), theme::bold());
            lines.push(Line::raw(""));
            let mut info = format!(
                "{} · {}",
                day_word(&local, &there),
                fmt::utc_offset(there.offset().seconds())
            );
            if is_here(app, &selected.zone_name) {
                info += " · here";
            }
            lines.push(Line::styled(format!("   {info}"), theme::dim()));
            let right = Line::styled(there.strftime("%A %-d %B").to_string(), theme::dim());
            pane(
                frame,
                top,
                widgets::pane(&selected.name, false, None, Some(right)),
                lines,
            );
        }
        None => {
            let lines = vec![
                Line::raw(""),
                Line::styled(
                    format!("   '{}' is not a time zone.", selected.zone_name),
                    theme::err(),
                ),
            ];
            pane(
                frame,
                top,
                widgets::pane(&selected.name, false, None, None),
                lines,
            );
        }
    }

    let rows: Vec<Line> = app
        .places
        .iter()
        .map(|place| match &place.zone {
            Some(zone) => {
                let there = zoned(now, zone);
                let day = match day_word(&local, &there) {
                    "today" => String::new(),
                    word => format!("{} · {word}", there.strftime("%a")),
                };
                let gap = there.offset().seconds() - local.offset().seconds();
                Line::from(vec![
                    Span::styled(widgets::fit(&place.name, 18), theme::text()),
                    Span::styled(
                        widgets::fit(&there.strftime("%H:%M").to_string(), 8),
                        theme::bold(),
                    ),
                    Span::styled(widgets::fit(&day, 17), theme::dim()),
                    Span::styled(widgets::fit(&fmt::offset_difference(gap), 8), theme::dim()),
                    Span::styled(fmt::utc_offset(there.offset().seconds()), theme::dim()),
                ])
            }
            None => Line::from(vec![
                Span::styled(widgets::fit(&place.name, 18), theme::text()),
                Span::styled(format!("unknown zone '{}'", place.zone_name), theme::err()),
            ]),
        })
        .collect();
    let block = widgets::pane("Cities", true, Some(rows.len().to_string()), None);
    let inner = block.inner(list);
    frame.render_widget(block, list);
    for (index, rect) in widgets::rows(frame, inner, rows, Some(app.city)) {
        app.hits.add(rect, Click::Row(index));
    }
}

fn is_here(app: &App, zone_name: &str) -> bool {
    app.tz.iana_name() == Some(zone_name)
}

/// Where a moment there falls relative to the local calendar day.
fn day_word(local: &jiff::Zoned, there: &jiff::Zoned) -> &'static str {
    match local.date().until(there.date()).map(|s| s.get_days()) {
        Ok(1) => "tomorrow",
        Ok(-1) => "yesterday",
        _ => "today",
    }
}

// --- alarms ---

pub fn days_label(days: &[u8]) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    match days {
        [] => "Once".into(),
        [1, 2, 3, 4, 5, 6, 7] => "Every day".into(),
        [1, 2, 3, 4, 5] => "Weekdays".into(),
        [6, 7] => "Weekends".into(),
        days => days
            .iter()
            .filter_map(|d| NAMES.get(usize::from(*d).wrapping_sub(1)))
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn draw_alarms(app: &App, frame: &mut Frame, area: Rect) {
    let now = app.now();
    if app.state.alarms.is_empty() {
        empty_state(
            frame,
            area,
            "Alarms",
            [
                "No alarms. Press n to add one.",
                "They ring only while the computer is awake.",
            ],
        );
        return;
    }
    let rows: Vec<Line> = app
        .state
        .alarms
        .iter()
        .map(|a| {
            let (body, next) = if a.enabled {
                (
                    theme::text(),
                    a.due_ms().map_or(String::new(), |d| fmt::until(d - now)),
                )
            } else {
                (theme::dim(), String::new())
            };
            let name = if a.name.is_empty() { "Alarm" } else { &a.name };
            let switch = if a.enabled {
                Span::styled("● on", theme::ok())
            } else {
                Span::styled("○ off", theme::dim())
            };
            Line::from(vec![
                Span::styled(
                    widgets::fit(&format!("{:02}:{:02}", a.hour, a.minute), 8),
                    body.bold(),
                ),
                Span::styled(widgets::fit(name, 20), body),
                Span::styled(widgets::fit(&days_label(&a.days), 14), theme::dim()),
                Span::styled(widgets::fit(&next, 14), theme::dim()),
                switch,
            ])
        })
        .collect();
    let block = widgets::pane("Alarms", true, Some(rows.len().to_string()), None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    for (index, rect) in widgets::rows(frame, inner, rows, Some(app.alarm)) {
        app.hits.add(rect, Click::Row(index));
    }
}

// --- dialogs ---

/// Like `widgets::dialog`, and a click outside it closes it.
fn open_dialog(app: &App, frame: &mut Frame, title: &str, width: u16, height: u16) -> Rect {
    let inner = widgets::dialog(frame, title, width, height);
    app.hits.add(frame.area(), Click::Outside);
    app.hits.add(inner.outer(Margin::new(1, 1)), Click::Inside);
    inner
}

fn padded_hint(bindings: &[(&str, &str)]) -> Line<'static> {
    indent(widgets::hint(bindings), 2)
}

fn draw_new_timer(app: &App, frame: &mut Frame, input: &telmo_kit::input::TextInput) {
    let now = app.now();
    let inner = open_dialog(app, frame, "New timer", 56, 12);
    let preview = match parse::timer(&input.value) {
        Ok((ms, name)) => {
            let label = if name.is_empty() {
                String::new()
            } else {
                format!("{name} · ")
            };
            Line::styled(
                format!(
                    "  {label}{} · ends {}",
                    fmt::words(ms),
                    when(now + ms, now, &app.tz)
                ),
                theme::ok(),
            )
        }
        Err(parse::ParseError::Empty) => Line::raw(""),
        Err(e) => Line::styled(format!("  {e}"), theme::err()),
    };
    let mut field = vec![Span::raw("  ")];
    field.extend(input.spans(inner.width.saturating_sub(4) as usize, true));
    let mut recent = vec![Span::styled("  recent", theme::dim())];
    for ms in &app.state.recent {
        recent.push(Span::styled(
            format!("  {}", fmt::short(*ms)),
            theme::text(),
        ));
    }
    let lines = vec![
        Line::raw(""),
        Line::styled("  Try 5m, 1h30, 90s or 25:00, then a name.", theme::dim()),
        Line::raw(""),
        Line::from(field),
        Line::raw(""),
        preview,
        Line::raw(""),
        if app.state.recent.is_empty() {
            Line::raw("")
        } else {
            Line::from(recent)
        },
        Line::raw(""),
        padded_hint(&[("↵", "start"), ("tab", "recent"), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_new_alarm(app: &App, frame: &mut Frame, input: &telmo_kit::input::TextInput) {
    let now = app.now();
    let inner = open_dialog(app, frame, "New alarm", 56, 13);
    let preview = match parse::alarm(&input.value) {
        Ok(spec) => {
            let next = model::next_occurrence(spec.hour, spec.minute, &spec.days, now, &app.tz);
            let next = next.map_or(String::new(), |n| {
                format!(" · next {}", fmt::until(n - now))
            });
            Line::styled(
                format!(
                    "  {} at {:02}:{:02}{next}",
                    days_label(&spec.days),
                    spec.hour,
                    spec.minute
                ),
                theme::ok(),
            )
        }
        Err(parse::ParseError::Empty) => Line::raw(""),
        Err(e) => Line::styled(format!("  {e}"), theme::err()),
    };
    let mut field = vec![Span::raw("  ")];
    field.extend(input.spans(inner.width.saturating_sub(4) as usize, true));
    let lines = vec![
        Line::raw(""),
        Line::styled("  Time, then days if you like, then a name.", theme::dim()),
        Line::raw(""),
        Line::styled(
            "  Try 7:30 weekdays wake up, 6pm mon wed fri,",
            theme::dim(),
        ),
        Line::styled("  or 21:00 daily pills.", theme::dim()),
        Line::raw(""),
        Line::from(field),
        Line::raw(""),
        preview,
        Line::raw(""),
        padded_hint(&[("↵", "add"), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_help(app: &App, frame: &mut Frame) {
    let rows = [
        ("← →", "switch tab (or 1 to 4)"),
        ("↑ ↓", "move"),
        ("n", "new timer or alarm"),
        ("↵", "pause, resume or restart a timer; alarm on/off"),
        ("r", "restart a timer; reset the stopwatch"),
        ("space", "start or stop the stopwatch"),
        ("l", "lap"),
        ("x", "delete"),
        ("esc", "close"),
    ];
    let inner = open_dialog(app, frame, "Keys", 66, rows.len() as u16 + 8);
    let mut lines = vec![Line::raw("")];
    for (key, label) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("   {}", widgets::fit(key, 7)), theme::accent()),
            Span::styled(label, theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "   Everything keeps running with this closed.",
        theme::dim(),
    ));
    lines.push(Line::styled(
        "   Alarms ring only while the computer is awake.",
        theme::dim(),
    ));
    widgets::text(frame, inner, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, model::Alarm};
    use crossterm::event::{KeyEvent, KeyModifiers};
    use telmo_kit::App as _;

    const W: u16 = 90;
    const H: u16 = 22;

    fn zone(name: &str) -> TimeZone {
        TimeZone::get(name).expect("zone")
    }

    /// Saturday 2026-10-10, 14:07 in Buenos Aires.
    fn now() -> i64 {
        "2026-10-10T14:07[America/Argentina/Buenos_Aires]"
            .parse::<jiff::Zoned>()
            .expect("now")
            .timestamp()
            .as_millisecond()
    }

    fn fixture() -> App {
        let tz = zone("America/Argentina/Buenos_Aires");
        let now = now();
        let mut state = model::ClockState::default();
        // Tea has run for 28 s of 5 minutes; the others are set by hand.
        state.add_timer("Tea", 300_000, now - 28_000);
        state.add_timer("Pasta", 600_000, now - 110_000);
        state.add_timer("Pomodoro", 1_500_000, now - 375_000);
        state.timers[2].pause(now);
        state.add_timer("Laundry", 2_700_000, now - 2_700_000);
        state.timers[3].fired = true;
        state.recent = vec![1_500_000, 300_000, 5_400_000];
        // 12:34.5 on the clock, three laps behind it.
        state.stopwatch.started_at_ms = Some(now - 754_500);
        state.stopwatch.laps = vec![252_300, 485_810, 673_070];
        for (hour, minute, name, days) in [
            (7, 30, "Wake up", vec![1, 2, 3, 4, 5]),
            (18, 0, "Gym", vec![1, 3, 5]),
            (21, 0, "Pills", vec![1, 2, 3, 4, 5, 6, 7]),
        ] {
            state.add_alarm(
                Alarm {
                    name: name.into(),
                    hour,
                    minute,
                    days,
                    ..Alarm::default()
                },
                now,
                &tz,
            );
        }
        let mut off = Alarm {
            name: "Flight".into(),
            hour: 4,
            minute: 15,
            enabled: false,
            ..Alarm::default()
        };
        off.next_ms = None;
        state.add_alarm(off, now, &tz);
        let places = Config::default().places();
        let mut app = App::new(state, places, tz, false);
        app.fixed_now = Some(now);
        app
    }

    fn render(app: &App) -> String {
        telmo_kit::test::render(W, H, |frame| draw(app, frame))
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn a_failed_save_is_not_reported_as_done() {
        let mut app = App::new(model::ClockState::default(), Vec::new(), zone("UTC"), true);
        // SAFETY: no other test in this crate reads the state directory.
        unsafe { std::env::set_var("XDG_STATE_HOME", "/dev/null/telmo-state") };
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "5m tea");
        press(&mut app, KeyCode::Enter);
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
        let toast = app.toast.as_ref().expect("a toast");
        assert!(toast.message.starts_with("Can't save"), "{}", toast.message);
        assert!(app.state.timers.is_empty());
    }

    #[test]
    fn timers() {
        let app = fixture();
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn timers_empty() {
        let mut app = fixture();
        app.state.timers.clear();
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn new_timer() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "25m tea");
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn new_timer_with_a_typo() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "5x");
        let out = render(&app);
        assert!(out.contains("Can't read '5x' as a length."));
        press(&mut app, KeyCode::Enter);
        assert!(app.dialog.is_some(), "a bad length keeps the dialog open");
        assert_eq!(app.state.timers.len(), 4);
    }

    #[test]
    fn starting_a_timer_adds_it_and_remembers_the_length() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "90s soup");
        press(&mut app, KeyCode::Enter);
        assert!(app.dialog.is_none());
        let timer = app.state.timers.last().expect("timer");
        assert_eq!((timer.name.as_str(), timer.duration_ms), ("soup", 90_000));
        assert_eq!(app.state.recent[0], 90_000);
        assert_eq!(app.timer, 4, "the new timer is selected");
    }

    #[test]
    fn tab_offers_recent_lengths_and_keeps_the_name() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "1m eggs");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        let Some(Dialog::NewTimer { input, .. }) = &app.dialog else {
            panic!("dialog")
        };
        assert_eq!(input.value, "5m eggs");
    }

    #[test]
    fn timer_keys() {
        let mut app = fixture();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.timers[0].status(now()), TimerStatus::Paused);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.timers[0].status(now()), TimerStatus::Running);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.state.timers.len(), 3);
        assert_eq!(app.state.timers[1].name, "Pomodoro");
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.tab, Tab::World);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.tab, Tab::Alarms, "tabs wrap around");
    }

    #[test]
    fn stopwatch() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('2'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn stopwatch_idle() {
        let mut app = fixture();
        app.state.stopwatch = model::Stopwatch::default();
        press(&mut app, KeyCode::Char('2'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn stopwatch_keys() {
        let mut app = fixture();
        app.state.stopwatch = model::Stopwatch::default();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.state.stopwatch.running());
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.state.stopwatch.laps.len(), 1);
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.state.stopwatch.running());
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.state.stopwatch, model::Stopwatch::default());
    }

    #[test]
    fn world() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('3'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn world_other_city() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('3'));
        for _ in 0..4 {
            press(&mut app, KeyCode::Down);
        }
        let out = render(&app);
        assert!(out.contains("Tokyo"));
        assert!(out.contains("tomorrow · UTC+9"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn alarms() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('4'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn new_alarm() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "7:30 weekdays wake up");
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn alarm_keys() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('4'));
        // Sorted by time of day: Flight, Wake up, Gym, Pills.
        assert_eq!(app.state.alarms[0].name, "Flight");
        press(&mut app, KeyCode::Enter);
        assert!(app.state.alarms[0].enabled && app.state.alarms[0].next_ms.is_some());
        press(&mut app, KeyCode::Enter);
        assert!(!app.state.alarms[0].enabled && app.state.alarms[0].next_ms.is_none());
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.state.alarms.len(), 3);
        press(&mut app, KeyCode::Char('n'));
        type_text(&mut app, "6pm mon wed gym");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.alarms.len(), 4);
        assert_eq!(app.state.alarms[app.alarm].name, "gym");
    }

    #[test]
    fn help() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('?'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn day_labels() {
        assert_eq!(days_label(&[]), "Once");
        assert_eq!(days_label(&[1, 2, 3, 4, 5]), "Weekdays");
        assert_eq!(days_label(&[1, 3, 5]), "Mon Wed Fri");
        assert_eq!(days_label(&[6, 7]), "Weekends");
        assert_eq!(days_label(&[1, 2, 3, 4, 5, 6, 7]), "Every day");
    }
}
