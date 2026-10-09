//! Drawing only: a pure function of the app state.

use crate::actions::Cmd;
use crate::app::{App, Click, Dialog, SWITCH_TOAST};
use crate::rebuild::Status;
use crate::tracker;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

/// JetBrainsMono Nerd Font icons: lock, moon with zz, restart arrow, power,
/// log out, hammer, stop sign.
const LOCK: &str = "\u{f023}";
const SLEEP: &str = "\u{f0904}";
const RESTART: &str = "\u{f0709}";
const POWER: &str = "\u{23fb}";
const LOG_OUT: &str = "\u{f0343}";
const REBUILD: &str = "\u{f08ea}";
const QUIT: &str = "\u{f015c}";

const KEYS: [(&str, &str); 8] = [
    ("l", LOCK),
    ("s", SLEEP),
    ("r", RESTART),
    ("p", POWER),
    ("o", LOG_OUT),
    ("u", REBUILD),
    ("k", QUIT),
    ("?", ""),
];
/// Columns between key pairs.
const GAP: usize = 3;
const BACKGROUND: Color = Color::Rgb(22, 23, 34);

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    let area = frame.area();
    app.area.set((area.width, area.height));
    if area.height == 0 {
        return;
    }
    draw_canvas(app, frame);
    if let Some(view) = &app.view {
        crate::apps::draw::draw(app, view, frame);
        return;
    }
    app.hits.add(logo_area(app), Click::Logo);
    if area.height > 1 {
        draw_switch_toast(app, frame, area.y + area.height - 2);
        if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
            let row = Rect {
                y: area.y + area.height - 2,
                height: 1,
                ..area
            };
            toast.render(frame, row);
        }
    }
    draw_footer(app, frame, area.y + area.height - 1);
    if let Some(dialog) = app.dialog {
        match dialog {
            Dialog::Help => draw_help(app, frame),
            Dialog::Confirm(cmd) => draw_confirm(app, frame, cmd),
        }
    }
}

/// Copies the effect's cells over the popup; empty cells stay untouched.
fn draw_canvas(app: &App, frame: &mut Frame) {
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in 0..app.canvas.height.min(area.height) {
        for x in 0..app.canvas.width.min(area.width) {
            let Some(cell) = app.canvas.get(x.into(), y.into()) else {
                continue;
            };
            let target = &mut buffer[(area.x + x, area.y + y)];
            target.set_char(cell.ch).set_fg(cell.fg);
            if cell.bold {
                target.modifier.insert(Modifier::BOLD);
            }
        }
    }
}

fn logo_area(app: &App) -> Rect {
    let (dx, dy) = app.logo.offset;
    Rect {
        x: (app.logo.ox + dx).max(0) as u16,
        y: (app.logo.oy + dy).max(0) as u16,
        width: app.logo.width.max(0) as u16,
        height: app.logo.height.max(0) as u16,
    }
}

/// A label and the space before it; `?` has none.
fn label_width(label: &str) -> usize {
    if label.is_empty() {
        0
    } else {
        1 + label.chars().count()
    }
}

/// The key pairs that fit in `room` columns, dropping from the end but
/// keeping `? more` for as long as possible.
fn fitting_keys(room: usize) -> Vec<(&'static str, &'static str)> {
    let width = |keys: &[(&str, &str)]| {
        let text: usize = keys
            .iter()
            .map(|(k, l)| k.chars().count() + label_width(l))
            .sum();
        text + GAP * keys.len().saturating_sub(1)
    };
    let mut keys = KEYS.to_vec();
    while width(&keys) > room && keys.len() > 1 {
        keys.remove(keys.len() - 2);
    }
    if width(&keys) > room {
        keys.clear();
    }
    keys
}

/// Rebuild progress or the success line replaces the whole footer.
fn draw_rebuild_footer(app: &App, frame: &mut Frame, y: u16) -> bool {
    let line = if let Some((status, elapsed)) = app.rebuild.progress(app.now) {
        progress_line(app, status, elapsed, frame.area().width)
    } else if let Some(text) = app.rebuild.banner(app.now) {
        Line::styled(text.to_string(), theme::ok().add_modifier(Modifier::BOLD))
    } else {
        return false;
    };
    let area = frame.area();
    let x = area.x + area.width.saturating_sub(line.width() as u16) / 2;
    let row = Rect {
        x,
        y,
        width: (line.width() as u16).min(area.width),
        height: 1,
    };
    frame.render_widget(line, row);
    true
}

const BAR_WIDTH: usize = 18;

fn progress_line(app: &App, status: &Status, elapsed: u64, width: u16) -> Line<'static> {
    let spinner = ["-", "\\", "|", "/"][(app.now * 8.0) as usize % 4];
    let mut spans = vec![
        Span::styled(format!("{spinner} "), Style::new().fg(theme::YELLOW)),
        Span::styled(format!("rebuilding {}   ", app.host), theme::text()),
    ];
    let clock = tracker::clock(elapsed);
    let counted = if status.to_build > 0 {
        Some((status.built, status.to_build, "built"))
    } else if status.to_fetch > 0 {
        Some((status.fetched, status.to_fetch, "fetched"))
    } else {
        None
    };
    match counted {
        Some((done, total, what)) => {
            let filled =
                (done.min(total) as usize * BAR_WIDTH + total as usize / 2) / total as usize;
            spans.push(Span::styled(
                "━".repeat(filled),
                Style::new().fg(theme::YELLOW),
            ));
            spans.push(Span::styled("─".repeat(BAR_WIDTH - filled), theme::faint()));
            spans.push(Span::styled(
                format!("   {done}/{total} {what} · {clock}"),
                theme::dim(),
            ));
        }
        None => {
            let room = (width as usize)
                .saturating_sub(spans.iter().map(Span::width).sum::<usize>() + clock.len() + 6);
            spans.push(Span::styled(
                format!("{} · {clock}", shorten(&status.last_line, room.min(48))),
                theme::dim(),
            ));
        }
    }
    Line::from(spans)
}

/// Cuts `text` to `max` characters, ending in `…` when it was longer.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn draw_footer(app: &App, frame: &mut Frame, y: u16) {
    if draw_rebuild_footer(app, frame, y) {
        return;
    }
    let area = frame.area();
    let name = app.prefs.effect.as_str();
    let name_width = name.chars().count() as u16;
    let right = area.width.saturating_sub(2 + name_width + 4);
    // Two columns of air between the keys and the effect name.
    let room = (right as usize).saturating_sub(1 + 2);
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in fitting_keys(room).iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".repeat(GAP)));
        }
        spans.push(Span::styled(*key, theme::accent()));
        if !label.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(*label, theme::dim()));
        }
    }
    let row = Rect {
        y,
        height: 1,
        ..area
    };
    frame.render_widget(Line::from(spans), row);

    if area.width < name_width + 6 {
        return;
    }
    let selector = Line::from(vec![
        Span::styled("‹", theme::accent()),
        Span::raw(" "),
        Span::styled(name, theme::bold()),
        Span::raw(" "),
        Span::styled("›", theme::accent()),
    ]);
    let cell = |x: u16| Rect {
        x: area.x + x,
        y,
        width: 1,
        height: 1,
    };
    frame.render_widget(
        selector,
        Rect {
            x: area.x + right,
            width: name_width + 4,
            ..row
        },
    );
    app.hits.add(cell(right), Click::Prev);
    app.hits.add(cell(right + 3 + name_width), Click::Next);
}

/// Moves a color toward the popup's dark base; `amount` 0 keeps it, 1 hides it.
fn fade(color: Color, amount: f32) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (color, BACKGROUND) else {
        return color;
    };
    let mix = |from: u8, to: u8| (from as f32 + (to as f32 - from as f32) * amount).round() as u8;
    Color::Rgb(mix(r, br), mix(g, bg), mix(b, bb))
}

/// The effect name and one dot per effect, for a moment after a switch.
fn draw_switch_toast(app: &App, frame: &mut Frame, y: u16) {
    let elapsed = app.now - app.switched_at;
    if !(0.0..SWITCH_TOAST).contains(&elapsed) {
        return;
    }
    let hidden = if elapsed < 1.0 {
        0.0
    } else {
        (elapsed - 1.0) / (SWITCH_TOAST - 1.0)
    };
    let name = app.prefs.effect.as_str();
    let mut spans = vec![Span::styled(
        format!("{name}   "),
        Style::new()
            .fg(fade(theme::FG, hidden))
            .add_modifier(Modifier::BOLD),
    )];
    for (i, effect) in app.cycle.iter().enumerate() {
        let (dot, color) = if *effect == app.prefs.effect {
            ("●", theme::BLUE)
        } else {
            ("○", theme::DIM)
        };
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(dot, Style::new().fg(fade(color, hidden))));
    }
    let area = frame.area();
    let line = Line::from(spans);
    let width = line.width() as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let row = Rect {
        x,
        y,
        width: width.min(area.width),
        height: 1,
    };
    frame.render_widget(line, row);
}

/// Like `widgets::dialog`, and a click outside it closes it.
pub(crate) fn open_dialog(
    app: &App,
    frame: &mut Frame,
    title: &str,
    width: u16,
    height: u16,
) -> Rect {
    let inner = widgets::dialog(frame, title, width, height);
    app.hits.add(frame.area(), Click::Outside);
    app.hits.add(inner.outer(Margin::new(1, 1)), Click::Inside);
    inner
}

fn draw_confirm(app: &App, frame: &mut Frame, cmd: Cmd) {
    let (title, verb) = match cmd {
        Cmd::Restart => (format!("Restart {}?", app.host), "restart"),
        Cmd::ShutDown => (format!("Shut down {}?", app.host), "shut down"),
        _ => ("Log out?".to_string(), "log out"),
    };
    let inner = open_dialog(app, frame, &title, 46, 6);
    let lines = vec![
        Line::raw(""),
        Line::styled("  Open apps will be asked to close.", theme::dim()),
        Line::raw(""),
        padded_hint(&[("↵", verb), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
    let hint = Rect {
        y: inner.y + 3,
        height: 1,
        x: inner.x + 1,
        width: inner.width.saturating_sub(1),
    };
    if let Some(area) = widgets::hint_areas(hint, &[("↵", verb), ("esc", "cancel")]).first() {
        app.hits.add(*area, Click::Confirm);
    }
}

/// Dialog hints sit two columns in, like the body text.
pub(crate) fn padded_hint(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut line = widgets::hint(bindings);
    line.spans.insert(0, Span::raw(" "));
    line
}

fn draw_help(app: &App, frame: &mut Frame) {
    let keys = [
        ("← →", "", "previous and next effect"),
        ("space", "", "switch between the Apple and Nix logo"),
        ("l", LOCK, "lock the screen"),
        ("s", SLEEP, "sleep"),
        ("r", RESTART, "restart (asks first)"),
        ("p", POWER, "shut down (asks first)"),
        ("o", LOG_OUT, "log out (asks first)"),
        ("u", REBUILD, "rebuild in the background"),
        ("L", "", "open the rebuild log"),
        ("k", QUIT, "force quit an app"),
        ("esc q", "", "close, then quit"),
    ];
    let inner = open_dialog(app, frame, "Keys", 56, keys.len() as u16 + 4);
    let mut lines = vec![Line::raw("")];
    for (key, icon, what) in keys {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(widgets::fit(key, 7), theme::accent()),
            Span::styled(widgets::fit(icon, 3), theme::dim()),
            Span::styled(what, theme::text()),
        ]));
    }
    widgets::text(frame, inner, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use telmo_kit::App as _;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn app(width: u16, height: u16) -> (App, UnboundedReceiver<Cmd>) {
        let (tx, rx) = unbounded_channel();
        (App::for_test(tx, (width, height)), rx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    use crossterm::event::KeyCode;

    fn render(app: &App, width: u16, height: u16) -> String {
        telmo_kit::test::render(width, height, |f| draw(app, f))
    }

    #[test]
    fn main_screen() {
        let (app, _rx) = app(90, 22);
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn narrow_footer() {
        let (app, _rx) = app(60, 22);
        insta::assert_snapshot!(render(&app, 60, 22));
    }

    #[test]
    fn switch_toast() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Right);
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn confirm_dialog() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('r'));
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn help_overlay() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('?'));
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn confirming_sends_the_command() {
        let (mut app, mut rx) = app(90, 22);
        press(&mut app, KeyCode::Char('p'));
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        assert_eq!(rx.try_recv(), Ok(Cmd::ShutDown));
    }

    #[test]
    fn escape_cancels_then_quits() {
        let (mut app, mut rx) = app(90, 22);
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            telmo_kit::Flow::Continue
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            telmo_kit::Flow::Quit
        );
    }

    #[test]
    fn arrows_wrap_around_the_cycle() {
        let (mut app, _rx) = app(90, 22);
        let first = app.prefs.effect.clone();
        press(&mut app, KeyCode::Left);
        assert_eq!(app.prefs.effect, *app.cycle.last().unwrap());
        press(&mut app, KeyCode::Right);
        assert_eq!(app.prefs.effect, first);
    }

    #[test]
    fn clicking_the_arrows_switches() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let (mut app, _rx) = app(90, 22);
        let first = app.prefs.effect.clone();
        render(&app, 90, 22);
        let right = 90 - 2 - 1;
        let click = |column| MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column,
            row: 21,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click(right));
        assert_ne!(app.prefs.effect, first);
    }

    /// Runs the app's clock forward in whole seconds.
    fn wait(app: &mut App, seconds: u32) {
        for _ in 0..seconds {
            app.advance(1.0);
        }
    }

    #[test]
    fn footer_while_waiting_for_touch_id() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        wait(&mut app, 1);
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn footer_while_rebuilding() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        wait(&mut app, 6);
        insta::assert_snapshot!(render(&app, 90, 22));
    }

    #[test]
    fn footer_after_success_then_back_to_keys() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        wait(&mut app, 13);
        assert!(!app.rebuild.running());
        insta::assert_snapshot!(render(&app, 90, 22));
        wait(&mut app, 5);
        assert!(render(&app, 90, 22).contains(&format!("l {LOCK}")));
    }

    #[test]
    fn rebuild_speeds_up_the_effect_and_finale_lasts_one_frame() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        wait(&mut app, 1);
        assert!(app.rebuild.running());
        wait(&mut app, 11);
        assert!(!app.rebuild.running());
        // The frame that saw the finish consumed the flag.
        assert!(!app.finished_pending());
    }

    #[test]
    fn a_second_u_says_one_is_running() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        press(&mut app, KeyCode::Char('u'));
        assert!(
            app.toast
                .as_ref()
                .is_some_and(|t| t.message == "A rebuild is already running.")
        );
    }

    #[test]
    fn mock_note_does_not_quit() {
        let (mut app, _rx) = app(90, 22);
        let flow = app.event(crate::actions::Event::Note("Would lock the screen.".into()));
        assert_eq!(flow, telmo_kit::Flow::Continue);
    }

    #[test]
    fn u_asks_for_a_sudo_command() {
        use std::os::unix::process::ExitStatusExt;
        let (mut app, _rx) = app(90, 22);
        app.use_real_rebuild(vec!["/bin/echo".into(), "switch".into()]);
        press(&mut app, KeyCode::Char('u'));
        let command = app.take_command().expect("u should ask for a command");
        let program = command.get_program().to_string_lossy().into_owned();
        assert!(program.ends_with("sudo"), "{program}");
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let line = args.join(" ");
        assert!(line.contains("rebuild-run --as-root --state-dir"), "{line}");
        assert!(line.ends_with("-- /bin/echo switch"), "{line}");
        assert!(app.take_command().is_none());

        app.command_finished(Ok(std::process::ExitStatus::from_raw(256)));
        assert!(!app.rebuild.running());
        assert!(
            app.toast
                .as_ref()
                .is_some_and(|t| t.message == crate::rebuild::AUTH_FAILED)
        );

        app.command_finished(Ok(std::process::ExitStatus::from_raw(0)));
        assert!(app.rebuild.running());
    }

    #[test]
    fn mock_u_never_asks_for_sudo() {
        let (mut app, _rx) = app(90, 22);
        press(&mut app, KeyCode::Char('u'));
        assert!(app.take_command().is_none());
    }
}
