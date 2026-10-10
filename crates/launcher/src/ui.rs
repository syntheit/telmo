//! Drawing: a pure function of the app state.

use crate::app::{App, CHROME, Click, Row};
use crate::math::Math;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders},
};
use ratatui_image::{FilterType, Image, Resize};
use telmo_kit::{theme, widgets};
use unicode_width::UnicodeWidthStr;

/// Where the name starts: bar, gap, two cells of icon, gap.
const NAME_X: u16 = 6;
const ICON_X: u16 = 3;
const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
/// The running dot of a row that isn't selected.
const DOT_DIM: Color = Color::Rgb(0x4f, 0x6b, 0x3a);
const BRIGHT: Color = Color::Rgb(0xe6, 0xe8, 0xff);

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    app.drawn_images.borrow_mut().clear();
    let area = frame.area();
    app.area.set((area.width, area.height));
    let boxed = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::accent());
    frame.render_widget(block, boxed);
    draw_query(app, frame, boxed);
    draw_separator(frame, boxed);
    let list = Rect {
        x: boxed.x + 1,
        y: boxed.y + 3,
        width: boxed.width.saturating_sub(2),
        height: area.height.saturating_sub(CHROME),
    };
    draw_rows(app, frame, list);
    let footer = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };
    match app.toast.as_ref().filter(|t| !t.expired()) {
        Some(toast) => toast.render(frame, footer),
        None => {
            widgets::keys(frame, footer, &key_bar(app));
        }
    }
    if let Some(power) = app.dialog {
        draw_confirm(frame, power, &app.host);
    }
}

fn draw_query(app: &App, frame: &mut Frame, boxed: Rect) {
    let query = app.query();
    let y = boxed.y + 1;
    let x = boxed.x + 2;
    let width = UnicodeWidthStr::width(query) as u16;
    frame.render_widget(
        Line::styled(query.to_string(), theme::bold()),
        Rect::new(x, y, boxed.width.saturating_sub(4), 1),
    );
    if query.is_empty() {
        let hint = "Search apps    ! commands    ? web";
        frame.render_widget(
            Line::styled(hint, theme::faint()),
            Rect::new(x + 2, y, boxed.width.saturating_sub(6), 1),
        );
    }
    let mode = app.mode_word();
    let mode_x = (boxed.x + boxed.width).saturating_sub(2 + mode.len() as u16);
    // Keep the mode word off long queries.
    if x + width + 2 < mode_x {
        frame.render_widget(
            Line::styled(mode, theme::faint()),
            Rect::new(mode_x, y, mode.len() as u16, 1),
        );
    }
    if app.dialog.is_none() {
        frame.set_cursor_position((x + width, y));
    }
}

fn draw_separator(frame: &mut Frame, boxed: Rect) {
    let y = boxed.y + 2;
    let buf = frame.buffer_mut();
    for x in boxed.x + 1..boxed.x + boxed.width.saturating_sub(1) {
        buf[(x, y)].set_symbol("─").set_style(theme::faint());
    }
    buf[(boxed.x, y)].set_symbol("├").set_style(theme::accent());
    buf[(boxed.x + boxed.width - 1, y)]
        .set_symbol("┤")
        .set_style(theme::accent());
}

fn draw_rows(app: &App, frame: &mut Frame, list: Rect) {
    if app.rows.is_empty() {
        let text = match app.query().trim_start().chars().next() {
            Some('!') => "No command matches that.",
            _ => "Nothing matches. ! for commands, ? to search the web.",
        };
        frame.render_widget(
            Line::styled(text, theme::dim()),
            Rect::new(list.x + 2, list.y, list.width.saturating_sub(2), 1),
        );
        return;
    }
    let mut y = list.y;
    for (index, row) in app.rows.iter().enumerate() {
        if y >= list.y + list.height {
            break;
        }
        let selected = index == app.selected;
        let line = Rect {
            y,
            height: 1,
            ..list
        };
        app.hits.add(
            Rect {
                height: row.lines() as u16,
                ..line
            },
            Click::Row(index),
        );
        if selected {
            frame
                .buffer_mut()
                .set_style(line, Style::new().bg(theme::SELECTION));
            frame.render_widget(
                Line::styled("▌", Style::new().fg(theme::BLUE).bg(theme::SELECTION)),
                Rect { width: 1, ..line },
            );
        }
        match row {
            Row::App { app: i, hits } => draw_app(app, frame, line, *i, hits, selected),
            Row::Math(math) => draw_math(frame, line, math, selected),
            Row::Command { command, hits } => {
                let c = &app.commands[*command];
                draw_titled(
                    frame, line, c.glyph, c.color, &c.label, hits, None, selected,
                );
            }
            Row::Search { engine } => draw_search(app, frame, line, *engine, selected),
            Row::Timer { duration, name } => {
                draw_timer(app, frame, line, duration, name.as_deref(), selected)
            }
        }
        y += row.lines() as u16;
    }
}

fn bg(selected: bool) -> Style {
    if selected {
        Style::new().bg(theme::SELECTION)
    } else {
        Style::new()
    }
}

/// The name with the matched letters picked out, cut to `width` cells.
fn name_spans(name: &str, hits: &[u32], width: usize, selected: bool) -> Vec<Span<'static>> {
    let base = if selected {
        Style::new().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        theme::text()
    };
    name.chars()
        .take(width)
        .enumerate()
        .map(|(i, c)| {
            let style = if hits.contains(&(i as u32)) {
                Style::new().fg(theme::YELLOW).add_modifier(Modifier::BOLD)
            } else {
                base
            };
            Span::styled(c.to_string(), style.patch(bg(selected)))
        })
        .collect()
}

fn right_text(frame: &mut Frame, line: Rect, text: &str, style: Style) {
    let width = UnicodeWidthStr::width(text) as u16;
    let x = (line.x + line.width).saturating_sub(width + 1);
    frame.render_widget(
        Line::styled(text.to_string(), style),
        Rect::new(x, line.y, width, 1),
    );
}

fn draw_app(app: &App, frame: &mut Frame, line: Rect, index: usize, hits: &[u32], selected: bool) {
    let entry = &app.apps[index];
    draw_icon(
        app,
        frame,
        Rect::new(line.x + ICON_X - 1, line.y, 2, 1),
        &entry.id,
    );
    let hotkey = app.hotkey_label(entry);
    let reserved = hotkey.as_ref().map_or(0, |h| h.chars().count() + 2);
    let running = app.running(entry);
    let room = usize::from(line.width)
        .saturating_sub(usize::from(NAME_X) + reserved + if running { 3 } else { 1 });
    let spans = name_spans(&entry.name, hits, room, selected);
    let used: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    frame.render_widget(
        Line::from(spans),
        Rect::new(line.x + NAME_X - 1, line.y, room as u16, 1),
    );
    if running {
        let dot = if selected { theme::GREEN } else { DOT_DIM };
        let x = line.x + NAME_X - 1 + used as u16 + 2;
        frame.render_widget(
            Line::styled("●", Style::new().fg(dot).patch(bg(selected))),
            Rect::new(x, line.y, 1, 1),
        );
    }
    if let Some(hotkey) = hotkey {
        let style = Style::new()
            .fg(if selected { theme::BLUE } else { theme::DIM })
            .patch(bg(selected));
        right_text(frame, line, &hotkey, style);
    }
}

fn draw_icon(app: &App, frame: &mut Frame, rect: Rect, id: &str) {
    // Under a dialog the list is dimmed text; a bright picture would not fit.
    if app.dialog.is_some() {
        return;
    }
    let Some(Some(image)) = app.images.get(id) else {
        return;
    };
    let resize = Resize::Fit(Some(FilterType::Triangle));
    let mut protocols = app.protocols.borrow_mut();
    if !protocols.contains_key(id) {
        match app
            .picker
            .new_protocol(image.clone(), rect.as_size(), resize)
        {
            Ok(protocol) => {
                protocols.insert(id.to_string(), protocol);
            }
            Err(_) => return,
        }
    }
    if let Some(protocol) = protocols.get(id) {
        frame.render_widget(Image::new(protocol), rect);
        app.drawn_images
            .borrow_mut()
            .push((rect.x, rect.y, id.to_string()));
    }
}

fn draw_math(frame: &mut Frame, line: Rect, math: &Math, selected: bool) {
    let sel = bg(selected);
    frame.render_widget(
        Line::styled(
            "=",
            Style::new()
                .fg(ORANGE)
                .add_modifier(Modifier::BOLD)
                .patch(sel),
        ),
        Rect::new(line.x + ICON_X - 1, line.y, 1, 1),
    );
    let value = Style::new()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
        .patch(sel);
    frame.render_widget(
        Line::styled(math.display.clone(), value),
        Rect::new(
            line.x + NAME_X - 1,
            line.y,
            line.width.saturating_sub(NAME_X + 11),
            1,
        ),
    );
    right_text(frame, line, "↵ copies", theme::dim().patch(sel));
    if !math.related.is_empty() {
        let text = math.related.join("  ·  ");
        frame.render_widget(
            Line::styled(text, theme::dim()),
            Rect::new(
                line.x + NAME_X - 1,
                line.y + 1,
                line.width.saturating_sub(NAME_X),
                1,
            ),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_titled(
    frame: &mut Frame,
    line: Rect,
    glyph: &str,
    color: Color,
    label: &str,
    hits: &[u32],
    right: Option<(&str, bool)>,
    selected: bool,
) {
    let sel = bg(selected);
    frame.render_widget(
        Line::styled(
            glyph.to_string(),
            Style::new()
                .fg(color)
                .add_modifier(Modifier::BOLD)
                .patch(sel),
        ),
        Rect::new(line.x + ICON_X - 1, line.y, 2, 1),
    );
    let reserved = right.map_or(0, |(t, _)| t.chars().count() + 2);
    let room = usize::from(line.width).saturating_sub(usize::from(NAME_X) + reserved + 1);
    frame.render_widget(
        Line::from(name_spans(label, hits, room, selected)),
        Rect::new(line.x + NAME_X - 1, line.y, room as u16, 1),
    );
    if let Some((text, _)) = right {
        let style = Style::new()
            .fg(if selected { theme::BLUE } else { theme::DIM })
            .patch(sel);
        right_text(frame, line, text, style);
    }
}

fn draw_search(app: &App, frame: &mut Frame, line: Rect, engine: usize, selected: bool) {
    let e = &app.engines[engine];
    let words = match crate::mode::Mode::parse(app.query()) {
        crate::mode::Mode::Web(rest) => crate::web::split(rest, &app.engines).1.to_string(),
        _ => String::new(),
    };
    let label = if words.is_empty() {
        e.label.clone()
    } else {
        format!("{}: {words}", e.label)
    };
    let tag = format!("?{}", e.key);
    draw_titled(
        frame,
        line,
        e.glyph,
        e.color,
        &label,
        &[],
        Some((&tag, false)),
        selected,
    );
}

fn draw_timer(
    app: &App,
    frame: &mut Frame,
    line: Rect,
    duration: &str,
    name: Option<&str>,
    selected: bool,
) {
    let label = if !app.clock {
        "Clock isn't installed".to_string()
    } else if let Some(name) = name {
        format!("Start a {duration} timer: {name}")
    } else {
        format!("Start a {duration} timer")
    };
    draw_titled(
        frame,
        line,
        "\u{f13ab}",
        theme::GREEN,
        &label,
        &[],
        None,
        selected,
    );
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.selected_row() {
        Some(Row::Math(_)) => vec![("↵", "copy"), ("tab", "with unit"), ("esc", "close")],
        Some(Row::Search { .. }) => vec![("↵", "search"), ("tab", "next engine"), ("esc", "close")],
        Some(Row::Command { .. } | Row::Timer { .. }) => vec![("↵", "run"), ("esc", "close")],
        Some(Row::App { .. }) => vec![
            ("↵", "open"),
            ("!", "commands"),
            ("?", "web"),
            ("esc", "close"),
        ],
        None => vec![("!", "commands"), ("?", "web"), ("esc", "close")],
    }
}

fn draw_confirm(frame: &mut Frame, power: crate::commands::Power, host: &str) {
    let verb = power.verb();
    let inner = widgets::dialog(frame, &power.question(host), 46, 6);
    let mut hint = widgets::hint(&[("↵", verb), ("esc", "cancel")]);
    hint.spans.insert(0, Span::raw(" "));
    let lines = vec![
        Line::raw(""),
        Line::styled("  Open apps will be asked to close.", theme::dim()),
        Line::raw(""),
        hint,
    ];
    widgets::text(frame, inner, lines);
    let _ = Margin::new(0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Cmd, Event, mock};
    use crate::commands::{Env, Power};
    use crate::config::Config;
    use crate::history::History;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::sync::mpsc::{Receiver, channel};
    use telmo_kit::{App as _, Flow};

    const W: u16 = 66;
    const H: u16 = 14;
    const NOW: u64 = 1_790_000_000;

    struct Fixture {
        app: App,
        rx: Receiver<Cmd>,
    }

    fn fixture() -> Fixture {
        let (tx, rx) = channel();
        let mut config = Config {
            hotkey_label: "fn".into(),
            ..Config::default()
        };
        for (key, name) in [
            ("t", "Ghostty"),
            ("w", "Zen Browser"),
            ("s", "Spotify"),
            ("g", "Telegram"),
            ("o", "Obsidian"),
            ("c", "Visual Studio Code"),
        ] {
            config.keys.insert(key.into(), name.into());
        }
        let mut history = History::default();
        for (i, name) in [
            "Visual Studio Code",
            "Obsidian",
            "Telegram",
            "Spotify",
            "Zen Browser",
            "Ghostty",
        ]
        .iter()
        .enumerate()
        {
            let id = mock::apps()
                .into_iter()
                .find(|a| a.name == *name)
                .unwrap()
                .id;
            history.record("", &id, NOW - 1000 + i as u64);
        }
        let env = Env {
            macos: true,
            clock: true,
        };
        let mut app = App::new(tx, config, history, Vec::new(), env, "swift".into());
        app.fixed_now = Some(NOW);
        app.area.set((W, H));
        app.event(Event::Apps(mock::apps()));
        app.event(Event::Running(mock::running()));
        for a in mock::apps() {
            let image = Some(mock::icon(&a.name));
            app.event(Event::Icon { id: a.id, image });
        }
        Fixture { app, rx }
    }

    fn press(f: &mut Fixture, code: KeyCode) -> Flow {
        f.app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(f: &mut Fixture, text: &str) {
        for c in text.chars() {
            press(f, KeyCode::Char(c));
        }
    }

    use crossterm::event::KeyCode;

    /// Icons are drawn as half blocks here; they are a placeholder, the layout is what counts.
    fn render(f: &Fixture) -> String {
        telmo_kit::test::render(W, H, |frame| draw(&f.app, frame))
            .chars()
            .map(|c| if "▀▄█".contains(c) { '▀' } else { c })
            .collect()
    }

    fn sent(f: &Fixture) -> Vec<Cmd> {
        f.rx.try_iter()
            .filter(|c| !matches!(c, Cmd::Icon(_)))
            .collect()
    }

    #[test]
    fn empty() {
        let f = fixture();
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn app_search() {
        let mut f = fixture();
        type_text(&mut f, "zen");
        let out = render(&f);
        assert!(out.contains("Zen Browser  ●"), "{out}");
        assert!(out.contains("fn W"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn initials_highlight() {
        let mut f = fixture();
        type_text(&mut f, "vsc");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn letter_key_puts_its_app_on_top() {
        let mut f = fixture();
        type_text(&mut f, "t");
        insta::assert_snapshot!(render(&f));
        match f.app.selected_row() {
            Some(Row::App { app, .. }) => assert_eq!(f.app.apps[*app].name, "Ghostty"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn math() {
        let mut f = fixture();
        type_text(&mut f, "2 ft to cm");
        let out = render(&f);
        assert!(out.contains("=  60.96 cm"), "{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn commands() {
        let mut f = fixture();
        type_text(&mut f, "!");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn commands_filtered() {
        let mut f = fixture();
        type_text(&mut f, "!sl");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn web() {
        let mut f = fixture();
        type_text(&mut f, "?nix ripgrep");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn restart_asks_first() {
        let mut f = fixture();
        type_text(&mut f, "!restart");
        press(&mut f, KeyCode::Enter);
        assert_eq!(f.app.dialog, Some(Power::Restart));
        assert!(sent(&f).is_empty(), "nothing runs before the answer");
        insta::assert_snapshot!(render(&f));
        press(&mut f, KeyCode::Enter);
        assert!(matches!(
            &sent(&f)[..],
            [Cmd::Run(Action::Power(Power::Restart))]
        ));
    }

    #[test]
    fn escape_cancels_the_dialog_not_the_launcher() {
        let mut f = fixture();
        type_text(&mut f, "!shut");
        press(&mut f, KeyCode::Enter);
        assert_eq!(f.app.dialog, Some(Power::ShutDown));
        assert_eq!(press(&mut f, KeyCode::Esc), Flow::Continue);
        assert_eq!(f.app.dialog, None);
        assert_eq!(press(&mut f, KeyCode::Esc), Flow::Quit);
        assert!(sent(&f).is_empty());
    }

    #[test]
    fn no_results() {
        let mut f = fixture();
        type_text(&mut f, "qzx");
        insta::assert_snapshot!(render(&f));
    }

    use crate::commands::Action;

    #[test]
    fn enter_opens_the_app_and_teaches_the_query() {
        let mut f = fixture();
        type_text(&mut f, "vsc");
        press(&mut f, KeyCode::Enter);
        match &sent(&f)[..] {
            [Cmd::Open { app, query }] => {
                assert_eq!(app.name, "Visual Studio Code");
                assert_eq!(query, "vsc");
            }
            other => panic!("{other:?}"),
        }
        assert!(f.app.pending);
        assert_eq!(f.app.event(Event::Done(None)), Flow::Quit);
    }

    #[test]
    fn math_copies_the_number_and_tab_copies_the_unit() {
        let mut f = fixture();
        type_text(&mut f, "2 ft to cm");
        press(&mut f, KeyCode::Enter);
        assert!(matches!(&sent(&f)[..], [Cmd::Copy(t)] if t == "60.96"));
        f.app.pending = false;
        press(&mut f, KeyCode::Tab);
        assert!(matches!(&sent(&f)[..], [Cmd::Copy(t)] if t == "60.96 cm"));
    }

    #[test]
    fn web_search_opens_the_named_engine_and_tab_moves_on() {
        let mut f = fixture();
        type_text(&mut f, "?gh telmo");
        press(&mut f, KeyCode::Enter);
        assert!(
            matches!(&sent(&f)[..], [Cmd::OpenUrl(u)] if u == "https://github.com/search?q=telmo")
        );
        f.app.pending = false;
        type_text(&mut f, "x");
        press(&mut f, KeyCode::Tab);
        press(&mut f, KeyCode::Enter);
        assert!(
            matches!(&sent(&f)[..], [Cmd::OpenUrl(u)] if u.starts_with("https://search.nixos.org")),
            "next engine after gh is the default nix"
        );
    }

    #[test]
    fn a_search_without_words_says_so() {
        let mut f = fixture();
        type_text(&mut f, "?nix");
        press(&mut f, KeyCode::Enter);
        assert!(sent(&f).is_empty());
        assert!(f.app.toast.is_some());
    }

    #[test]
    fn timers_start_through_the_clock() {
        let mut f = fixture();
        type_text(&mut f, "!t 5m tea");
        press(&mut f, KeyCode::Enter);
        match &sent(&f)[..] {
            [Cmd::Run(Action::Timer { duration, name })] => {
                assert_eq!(duration, "5m");
                assert_eq!(name.as_deref(), Some("tea"));
            }
            other => panic!("{other:?}"),
        }
        let mut f = fixture();
        f.app.clock = false;
        type_text(&mut f, "!t 5m");
        assert!(render(&f).contains("Clock isn't installed"));
        press(&mut f, KeyCode::Enter);
        assert!(sent(&f).is_empty());
        assert!(f.app.toast.is_some());
    }

    #[test]
    fn the_selection_wraps_and_the_top_row_only_changes_with_the_query() {
        let mut f = fixture();
        type_text(&mut f, "te");
        let top = f.app.rows[0].clone();
        press(&mut f, KeyCode::Down);
        assert_eq!(f.app.selected, 1);
        press(&mut f, KeyCode::Up);
        press(&mut f, KeyCode::Up);
        assert_eq!(f.app.selected, f.app.rows.len() - 1);
        // A rescan that finds the same apps leaves the list and selection alone.
        press(&mut f, KeyCode::Up);
        let at = f.app.selected;
        f.app.event(Event::Apps(mock::apps()));
        assert_eq!(f.app.selected, at);
        assert_eq!(f.app.rows[0], top);
        f.app
            .event(Event::Running(std::collections::HashSet::new()));
        assert_eq!(f.app.rows[0], top, "running state never reorders");
    }

    #[test]
    fn keys_typed_before_the_apps_arrive_are_kept() {
        let (tx, _rx) = channel();
        let mut app = App::new(
            tx,
            Config::default(),
            History::default(),
            Vec::new(),
            Env::default(),
            "h".into(),
        );
        app.area.set((W, H));
        for c in "zen".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.query(), "zen");
        assert!(app.rows.is_empty());
        app.event(Event::Apps(mock::apps()));
        assert!(matches!(app.rows.first(), Some(Row::App { .. })));
    }

    #[test]
    fn a_replaced_or_removed_icon_clears_the_screen_once() {
        let (mut f, drew) = (fixture(), |app: &mut App, ids: &[&str]| {
            app.drawn_images.replace(
                ids.iter()
                    .enumerate()
                    .map(|(i, id)| (3, i as u16, id.to_string()))
                    .collect(),
            );
            app.take_clear()
        });
        assert!(!drew(&mut f.app, &["a", "b"]), "first pictures");
        assert!(!drew(&mut f.app, &["a", "b"]), "same pictures");
        assert!(
            !drew(&mut f.app, &["a", "b", "c"]),
            "one more is nothing stale"
        );
        assert!(drew(&mut f.app, &["a", "c"]), "one gone");
        assert!(drew(&mut f.app, &[]), "all gone");
        assert!(!drew(&mut f.app, &[]), "still none");
    }

    #[test]
    fn icons_are_asked_for_once_per_visible_app() {
        let (tx, rx) = channel();
        let mut app = App::new(
            tx,
            Config::default(),
            History::default(),
            mock::apps(),
            Env::default(),
            "h".into(),
        );
        app.area.set((W, H));
        app.input.value = "s".into();
        app.refresh(false);
        let first: Vec<_> = rx.try_iter().collect();
        assert!(!first.is_empty() && first.len() <= 9);
        app.refresh(false);
        assert_eq!(rx.try_iter().count(), 0, "already asked");
    }
}
