//! Drawing: a pure function of the app state.

use crate::app::{App, Click, Dialog};
use crate::color::Rgb;
use crate::content::file_name;
use crate::model::{Entry, Kind};
use crate::{detect, search, timefmt};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use ratatui_image::{FilterType, Image, Resize};
use telmo_kit::{
    theme,
    widgets::{self, fit},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SIDEBAR: u16 = 38;
const ICON_TEXT: &str = "\u{f0262}";
const ICON_IMAGE: &str = "\u{f02e9}";
const ICON_LINK: &str = "\u{f0337}";
const ICON_COLOR: &str = "\u{f03d8}";
const ICON_FILE: &str = "\u{f0214}";
const ICON_PIN: &str = "\u{f0403}";
const ICON_CODE: &str = "\u{f0169}";
const LABEL_WIDTH: usize = 9;
const SWATCH: Size = Size::new(24, 5);

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    app.drawn_image.replace(None);
    let screen = widgets::screen(frame.area());
    widgets::header(frame, screen.header, "Clipboard", header_status(app));
    let [side, panel] =
        Layout::horizontal([Constraint::Length(SIDEBAR), Constraint::Fill(1)]).areas(screen.body);
    draw_sidebar(app, frame, side);
    draw_panel(app, frame, panel);
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bar = key_bar(app);
    let labels: Vec<(&str, &str)> = bar.iter().map(|(k, l)| (*k, l.as_str())).collect();
    let areas = widgets::keys(frame, screen.keys, &labels);
    for ((key, _), area) in bar.iter().zip(areas) {
        if let Some(code) = key_code(key) {
            app.hits.add(area, Click::Key(code));
        }
    }
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, frame, dialog);
    }
}

fn header_status(app: &App) -> Line<'static> {
    let total = app.snapshot.entries.len();
    let text = if !app.loaded {
        String::new()
    } else if app.search.is_some() {
        format!("{} of {total}", app.ordered().len())
    } else if total == 0 {
        "empty".into()
    } else {
        let noun = if total == 1 { "item" } else { "items" };
        format!("{total} {noun} · last {} days", app.snapshot.max_days)
    };
    Line::styled(text, theme::dim())
}

// --- key bar ---

fn key_code(label: &str) -> Option<KeyCode> {
    let mut chars = label.chars();
    match (label, chars.next(), chars.next()) {
        ("↵", ..) => Some(KeyCode::Enter),
        ("esc", ..) => Some(KeyCode::Esc),
        (_, Some(c), None) => Some(KeyCode::Char(c)),
        _ => None,
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, String)> {
    let plain = |keys: &[(&'static str, &str)]| -> Vec<(&'static str, String)> {
        keys.iter().map(|(k, l)| (*k, l.to_string())).collect()
    };
    if app.search.is_some() {
        return plain(&[("↵", "copy"), ("↑↓", "move"), ("esc", "clear search")]);
    }
    let Some(entry) = app.selected_entry() else {
        return plain(&[("?", ""), ("esc", "close")]);
    };
    let mut keys = plain(&[("↵", "copy")]);
    match entry.kind {
        Kind::Link => keys.extend(plain(&[("o", "open")])),
        Kind::File => keys.extend(plain(&[("o", "open"), ("r", "reveal")])),
        Kind::Color => {
            if let Some([a, b]) = app.next_notations(entry) {
                keys.push(("h", format!("copy as {}/{}", a.name(), b.name())));
            }
        }
        _ => {}
    }
    let pin = if entry.pinned { "unpin" } else { "pin" };
    keys.extend(plain(&[
        ("p", pin),
        ("x", "delete"),
        ("/", "search"),
        ("?", ""),
    ]));
    keys
}

// --- sidebar ---

enum Row {
    Heading(&'static str),
    Item(usize),
}

fn draw_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::pane("History", true, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let list = match &app.search {
        Some(input) => {
            let line = Line::from(vec![
                Span::raw(" "),
                Span::styled("/ ", theme::accent()),
                Span::styled(input.value.clone(), theme::text()),
                Span::styled("█", theme::accent()),
            ]);
            frame.render_widget(line, Rect { height: 1, ..inner });
            Rect {
                y: inner.y + 2,
                height: inner.height.saturating_sub(2),
                ..inner
            }
        }
        None => inner,
    };
    let items = app.ordered();
    if items.is_empty() {
        if app.loaded {
            let message = if app.search.is_some() {
                "  No matches."
            } else {
                "  Nothing copied yet."
            };
            let at = if app.search.is_some() { 0 } else { 1 };
            let area = Rect {
                y: list.y + at,
                height: list.height.saturating_sub(at),
                ..list
            };
            frame.render_widget(Line::styled(message, theme::dim()), area);
        }
        return;
    }
    draw_list(app, frame, list, &items);
}

fn draw_list(app: &App, frame: &mut Frame, area: Rect, items: &[&Entry]) {
    let grouped = app.search.is_none() && items.iter().any(|e| e.pinned);
    let mut lines = Vec::new();
    for (i, entry) in items.iter().enumerate() {
        if grouped && i == 0 {
            lines.push(Row::Heading("Pinned"));
        }
        if grouped && !entry.pinned && (i == 0 || items[i - 1].pinned) {
            lines.push(Row::Heading("Recent"));
        }
        lines.push(Row::Item(i));
    }
    let selected = lines
        .iter()
        .position(|l| matches!(l, Row::Item(i) if Some(&items[*i].id) == app.selected.as_ref()));
    let height = area.height as usize;
    let offset = selected.map_or(0, |s| (s + 1).saturating_sub(height));
    for (row, line) in lines.iter().enumerate().skip(offset).take(height) {
        let rect = Rect {
            y: area.y + (row - offset) as u16,
            height: 1,
            ..area
        };
        match line {
            Row::Heading(name) => {
                frame.render_widget(Line::styled(format!("  {name}"), theme::dim()), rect);
            }
            Row::Item(i) => {
                let entry = items[*i];
                let is_selected = selected == Some(row);
                if is_selected {
                    frame
                        .buffer_mut()
                        .set_style(rect, Style::new().bg(theme::SELECTION));
                }
                frame.render_widget(item_row(app, entry, rect.width as usize, is_selected), rect);
                app.hits.add(rect, Click::Row(entry.id.clone()));
            }
        }
    }
}

fn item_row(app: &App, entry: &Entry, width: usize, selected: bool) -> Line<'static> {
    let marker = if selected {
        Span::styled(" ▌ ", theme::accent())
    } else {
        Span::raw("   ")
    };
    let (icon, icon_color) = icon(entry);
    let (right, right_style) = if app.pending.as_deref() == Some(&entry.id) {
        (widgets::spinner(app.tick).to_string(), theme::accent())
    } else if entry.pinned {
        (ICON_PIN.to_string(), theme::warn())
    } else {
        (timefmt::short(app.now(), entry.copied_at), theme::dim())
    };
    // marker, icon and its space, a space before the right-hand column
    let room = width.saturating_sub(3 + 2 + 1 + right.width());
    let text = ellipsize(&list_text(entry), room);
    let padding = room.saturating_sub(text.width());
    let mut spans = vec![
        marker,
        Span::styled(icon, Style::new().fg(icon_color)),
        Span::raw(" "),
    ];
    spans.extend(highlighted(&text, app.query(), theme::text()));
    spans.push(Span::raw(" ".repeat(padding + 1)));
    spans.push(Span::styled(right, right_style));
    Line::from(spans)
}

fn icon(entry: &Entry) -> (&'static str, Color) {
    match entry.kind {
        Kind::Text => (ICON_TEXT, theme::DIM),
        Kind::Image => (ICON_IMAGE, theme::MAGENTA),
        Kind::Link => (ICON_LINK, theme::CYAN),
        Kind::Color => {
            let color =
                Rgb::parse(&entry.preview).map_or(theme::BLUE, |Rgb(r, g, b)| Color::Rgb(r, g, b));
            (ICON_COLOR, color)
        }
        Kind::File => (ICON_FILE, theme::YELLOW),
        Kind::Code => (ICON_CODE, theme::GREEN),
    }
}

/// What a list row shows of an item. Links lose their scheme.
fn list_text(entry: &Entry) -> String {
    let text = one_line(&entry.preview);
    if entry.kind != Kind::Link {
        return text;
    }
    for scheme in ["https://", "http://"] {
        if let Some(rest) = text.strip_prefix(scheme) {
            return rest.to_string();
        }
    }
    text
}

/// Newlines and runs of spaces become single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cuts to `width` columns, ending in `…` when something was dropped.
fn ellipsize(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// `text` in `base`, with the parts matching `query` stood out.
fn highlighted(text: &str, query: &str, base: Style) -> Vec<Span<'static>> {
    let ranges = search::ranges(text, query);
    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let chars: Vec<char> = text.chars().collect();
    let hit = base.fg(theme::YELLOW).bold();
    let mut spans = Vec::new();
    let mut at = 0;
    for range in ranges {
        if range.start > at {
            spans.push(Span::styled(
                chars[at..range.start].iter().collect::<String>(),
                base,
            ));
        }
        spans.push(Span::styled(
            chars[range.clone()].iter().collect::<String>(),
            hit,
        ));
        at = range.end;
    }
    if at < chars.len() {
        spans.push(Span::styled(chars[at..].iter().collect::<String>(), base));
    }
    spans
}

// --- panel ---

fn kind_title(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "Text",
        Kind::Link => "Link",
        Kind::Color => "Color",
        Kind::Code => "Code",
        Kind::Image => "Image",
        Kind::File => "File",
    }
}

fn draw_panel(app: &App, frame: &mut Frame, area: Rect) {
    let entry = app.selected_entry();
    let title = entry.map_or("Clipboard", |e| kind_title(e.kind));
    let block = widgets::pane(title, false, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content = inner.inner(Margin::new(2, 0));
    let Some(entry) = entry else {
        draw_nothing(app, frame, content);
        return;
    };
    let info = info_rows(app, entry);
    let rule_y = inner
        .bottom()
        .saturating_sub(info.len() as u16 + 1)
        .max(inner.y);
    frame.render_widget(
        Line::styled("─".repeat(content.width as usize), theme::faint()),
        Rect {
            y: rule_y,
            height: 1,
            ..content
        },
    );
    for (i, line) in info.into_iter().enumerate() {
        let rect = Rect {
            y: rule_y + 1 + i as u16,
            height: 1,
            ..content
        };
        frame.render_widget(line, rect);
    }
    let body = Rect {
        y: inner.y + 1,
        height: rule_y.saturating_sub(inner.y + 2),
        ..content
    };
    if body.height == 0 {
        return;
    }
    match entry.kind {
        Kind::Text | Kind::Code => draw_text(app, frame, body, entry),
        Kind::Link => draw_link(app, frame, body, entry),
        Kind::Color => draw_color(app, frame, body, entry),
        Kind::File => draw_files(app, frame, body, entry),
        Kind::Image => draw_image(app, frame, body, entry),
    }
}

fn draw_nothing(app: &App, frame: &mut Frame, content: Rect) {
    let body = Rect {
        y: content.y + 1,
        height: content.height.saturating_sub(1),
        ..content
    };
    if !app.loaded {
        return;
    }
    if app.search.is_some() {
        let text = format!("Nothing matches \"{}\".", app.query());
        frame.render_widget(Line::styled(text, theme::dim()), body);
        return;
    }
    let config = &app.snapshot;
    let lines = vec![
        Line::styled(
            "Everything you copy shows up here: text, links,",
            theme::text(),
        ),
        Line::styled("images, colors and files.", theme::text()),
        Line::raw(""),
        Line::styled(
            format!(
                "Keeps the last {} items for {} days.",
                config.max_items, config.max_days
            ),
            theme::dim(),
        ),
        Line::styled("Pins stay until you remove them.", theme::dim()),
    ];
    frame.render_widget(Paragraph::new(lines), body);
}

fn info_rows(app: &App, entry: &Entry) -> Vec<Line<'static>> {
    let mut when = timefmt::long(app.now(), entry.copied_at);
    if !entry.source.is_empty() {
        when = format!("{when} · {}", entry.source);
    }
    let mut rows = vec![info("Copied", when)];
    if let Some(size) = size_text(entry) {
        rows.push(info("Size", size));
    }
    rows
}

fn info(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(fit(label, LABEL_WIDTH), theme::dim()),
        Span::styled(value, theme::text()),
    ])
}

fn size_text(entry: &Entry) -> Option<String> {
    match entry.kind {
        Kind::Color => None,
        Kind::Link => Some(plural(entry.chars, "character", "characters")),
        Kind::Text | Kind::Code => Some(format!(
            "{} · {} · {}",
            plural(entry.chars, "character", "characters"),
            plural(entry.words, "word", "words"),
            plural(entry.lines, "line", "lines"),
        )),
        Kind::Image => Some(format!(
            "{} × {} · {} · {}",
            entry.width,
            entry.height,
            format_bytes(entry.bytes),
            entry.format
        )),
        Kind::File => Some(format!("{} · {}", format_bytes(entry.bytes), entry.format)),
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn draw_text(app: &App, frame: &mut Frame, area: Rect, entry: &Entry) {
    // Only what can be on screen is laid out, however long the text is.
    let limit = area.width as usize * area.height as usize * 2;
    let text: String = app.full_text(entry).chars().take(limit).collect();
    let lines: Vec<Line> = text
        .split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r').replace('\t', "    ");
            Line::from(highlighted(&line, app.query(), theme::text()))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_link(app: &App, frame: &mut Frame, area: Rect, entry: &Entry) {
    let url = app.full_text(entry).trim().to_string();
    let rows = url
        .chars()
        .count()
        .div_ceil(area.width.max(1) as usize)
        .max(1) as u16;
    let style = Style::new().fg(theme::CYAN);
    let line = Line::from(highlighted(&url, app.query(), style));
    frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), area);
    let (host, path) = detect::split_link(&url);
    let mut spans = vec![Span::styled(host, theme::bold())];
    if !path.is_empty() {
        spans.push(Span::styled("  ·  ", theme::faint()));
        spans.push(Span::styled(path, theme::dim()));
    }
    let y = area.y + rows + 1;
    if y < area.bottom() {
        let rect = Rect {
            y,
            height: 1,
            ..area
        };
        frame.render_widget(Line::from(spans), rect);
    }
}

fn draw_color(app: &App, frame: &mut Frame, area: Rect, entry: &Entry) {
    let text = app.full_text(entry).trim().to_string();
    let Some(rgb) = Rgb::parse(&text) else {
        draw_text(app, frame, area, entry);
        return;
    };
    let Rgb(r, g, b) = rgb;
    let swatch = Line::styled(
        "█".repeat(SWATCH.width as usize),
        Style::new().fg(Color::Rgb(r, g, b)),
    );
    for row in 0..SWATCH.height.min(area.height) {
        let rect = Rect {
            y: area.y + row,
            height: 1,
            ..area
        };
        frame.render_widget(swatch.clone(), rect);
    }
    let lines = vec![rgb.hex(), rgb.rgb(), rgb.hsl()];
    for (i, text) in lines.into_iter().enumerate() {
        let y = area.y + SWATCH.height + 1 + i as u16;
        if y < area.bottom() {
            let rect = Rect {
                y,
                height: 1,
                ..area
            };
            frame.render_widget(Line::styled(text, theme::text()), rect);
        }
    }
}

fn draw_files(app: &App, frame: &mut Frame, area: Rect, entry: &Entry) {
    let mut lines = Vec::new();
    let icon = Span::styled(format!("{ICON_FILE}  "), Style::new().fg(theme::YELLOW));
    if let [path] = entry.files.as_slice() {
        lines.push(Line::from(vec![
            icon,
            Span::styled(file_name(path).to_string(), theme::bold()),
        ]));
        lines.push(Line::styled(
            format!("   {}", folder(path, app.home.as_deref())),
            theme::dim(),
        ));
    } else {
        for path in &entry.files {
            lines.push(Line::from(vec![
                icon.clone(),
                Span::styled(file_name(path).to_string(), theme::bold()),
                Span::styled(
                    format!("  {}", folder(path, app.home.as_deref())),
                    theme::dim(),
                ),
            ]));
        }
    }
    lines.push(Line::raw(""));
    let finder = if cfg!(target_os = "macos") {
        "Finder"
    } else {
        "your file manager"
    };
    let verb = if entry.files.len() == 1 {
        "file"
    } else {
        "files"
    };
    lines.push(Line::styled(
        format!("Copying puts the {verb} back, ready to paste in {finder}."),
        theme::dim(),
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

/// The folder of a path, with the home directory as `~`.
fn folder(path: &str, home: Option<&str>) -> String {
    let parent = std::path::Path::new(path)
        .parent()
        .map_or_else(String::new, |p| p.display().to_string());
    match home {
        Some(home) if !home.is_empty() && parent == home => "~".into(),
        Some(home) if !home.is_empty() && parent.starts_with(&format!("{home}/")) => {
            format!("~{}", &parent[home.len()..])
        }
        _ => parent,
    }
}

fn draw_image(app: &App, frame: &mut Frame, area: Rect, entry: &Entry) {
    let dim = |text: &str| Line::styled(text.to_string(), theme::dim());
    let image = match app.images.get(&entry.id) {
        None => return frame.render_widget(dim("Loading…"), area),
        Some(None) => {
            return frame.render_widget(
                dim("Can't show this picture. It's still safe to copy."),
                area,
            );
        }
        Some(Some(image)) => image,
    };
    let resize = Resize::Scale(Some(FilterType::Triangle));
    let size = resize.size_for(
        image,
        app.picker.font_size(),
        Size::new(area.width, area.height),
    );
    let key = (entry.id.clone(), size.width, size.height);
    let mut protocols = app.protocols.borrow_mut();
    if !protocols.contains_key(&key) {
        if protocols.len() > 8 {
            protocols.clear();
        }
        match app.picker.new_protocol(image.clone(), size, resize) {
            Ok(protocol) => {
                protocols.insert(key.clone(), protocol);
            }
            Err(_) => return frame.render_widget(dim("Can't show this picture here."), area),
        }
    }
    if let Some(protocol) = protocols.get(&key) {
        let rect = Rect {
            width: size.width.min(area.width),
            height: size.height.min(area.height),
            ..area
        };
        frame.render_widget(Image::new(protocol), rect);
        app.drawn_image.replace(Some(key));
    }
}

// --- dialogs ---

fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    match dialog {
        Dialog::Delete(_) => confirm(
            frame,
            app,
            "Delete",
            (
                "Delete this from history?",
                "It stays on the clipboard if it's there now.",
            ),
            "delete",
        ),
        Dialog::ClearAll => confirm(
            frame,
            app,
            "Clear all",
            (
                "Clear the whole history?",
                "Pinned items go too. The clipboard itself stays.",
            ),
            "clear all",
        ),
        Dialog::Help => draw_help(app, frame),
    }
}

fn confirm(frame: &mut Frame, app: &App, title: &str, text: (&str, &str), action: &str) {
    let inner = widgets::dialog(frame, title, 54, 7);
    let bindings = [("↵", action), ("esc", "cancel")];
    let hint_area = Rect {
        x: inner.x + 2,
        y: inner.y + 5,
        width: inner.width.saturating_sub(2),
        height: 1,
    };
    let mut hint = widgets::hint(&bindings);
    hint.spans.insert(0, Span::raw("  "));
    let lines = vec![
        Line::raw(""),
        Line::styled(format!("   {}", text.0), theme::bold()),
        Line::styled(format!("   {}", text.1), theme::dim()),
        Line::raw(""),
        hint,
    ];
    widgets::text(frame, inner, lines);
    for ((key, _), area) in bindings
        .iter()
        .zip(widgets::hint_areas(hint_area, &bindings))
    {
        if let Some(code) = key_code(key) {
            app.hits.add(area, Click::DialogKey(code));
        }
    }
}

fn draw_help(app: &App, frame: &mut Frame) {
    let rows = [
        ("↑ ↓", "move"),
        ("↵", "put it back on the clipboard"),
        ("p", "pin or unpin"),
        ("x", "delete"),
        ("/", "search"),
        ("o", "open a link or file"),
        ("r", "reveal a file"),
        ("h", "copy a color as another notation"),
        ("c", "clear all history"),
        ("esc", "close"),
    ];
    let inner = widgets::dialog(frame, "Keys", 54, rows.len() as u16 + 4);
    let mut lines = vec![Line::raw("")];
    for (key, label) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("   {}", fit(key, 5)), theme::accent()),
            Span::styled(label, theme::dim()),
        ]));
    }
    widgets::text(frame, inner, lines);
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Event, mock};
    use crossterm::event::{KeyEvent, KeyModifiers};
    use telmo_kit::App as _;

    const NOW: u64 = 1_790_000_000;
    const W: u16 = 108;
    const H: u16 = 26;

    struct Fixture {
        app: App,
        _rx: std::sync::mpsc::Receiver<crate::backend::Cmd>,
    }

    fn fixture() -> Fixture {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx);
        app.fixed_now = Some(NOW);
        app.home = Some("/Users/daniel".into());
        app.event(Event::Snapshot(crate::model::Snapshot {
            entries: mock::entries(NOW),
            max_items: 50,
            max_days: 30,
        }));
        Fixture { app, _rx: rx }
    }

    fn press(f: &mut Fixture, code: KeyCode) {
        f.app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_text(f: &mut Fixture, text: &str) {
        for c in text.chars() {
            press(f, KeyCode::Char(c));
        }
    }

    fn select(f: &mut Fixture, id: &str) {
        f.app.selected = Some(id.into());
    }

    /// Half-block glyphs are the picture's pixels; the layout is what counts.
    fn render(f: &Fixture) -> String {
        telmo_kit::test::render(W, H, |frame| draw(&f.app, frame))
            .chars()
            .map(|c| if "▀▄█".contains(c) { '▀' } else { c })
            .collect()
    }

    #[test]
    fn text() {
        let f = fixture();
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn image() {
        let mut f = fixture();
        select(&mut f, "shot");
        f.app.event(Event::Image {
            id: "shot".into(),
            image: Some(mock::test_image()),
        });
        let out = render(&f);
        assert!(out.contains("Copied   5 min ago · Screenshot"));
        assert!(out.contains("Size     1440 × 900 · 412 KB · PNG"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn image_loading() {
        let mut f = fixture();
        select(&mut f, "shot");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn link() {
        let mut f = fixture();
        select(&mut f, "link");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn color() {
        let mut f = fixture();
        select(&mut f, "color");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn file() {
        let mut f = fixture();
        select(&mut f, "invoice");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn search() {
        let mut f = fixture();
        press(&mut f, KeyCode::Char('/'));
        type_text(&mut f, "nix");
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn delete() {
        let mut f = fixture();
        press(&mut f, KeyCode::Char('x'));
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn empty() {
        let mut f = fixture();
        f.app.event(Event::Snapshot(crate::model::Snapshot {
            entries: Vec::new(),
            max_items: 50,
            max_days: 30,
        }));
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn help() {
        let mut f = fixture();
        press(&mut f, KeyCode::Char('?'));
        insta::assert_snapshot!(render(&f));
    }

    #[test]
    fn no_matches() {
        let mut f = fixture();
        press(&mut f, KeyCode::Char('/'));
        type_text(&mut f, "zzz");
        let out = render(&f);
        assert!(out.contains("0 of 13"));
        assert!(out.contains("No matches."));
    }

    #[test]
    fn pinned_rows_use_the_pin_icon() {
        let f = fixture();
        let out = render(&f);
        assert!(out.contains(ICON_PIN));
        assert!(out.contains("Pinned"));
        assert!(out.contains("Recent"));
    }

    #[test]
    fn rows_end_in_the_time_column() {
        let f = fixture();
        let out = render(&f);
        assert!(
            out.lines()
                .any(|l| l.contains("hunter2-correct-hors… yesterday│"))
        );
    }

    #[test]
    fn ellipsize_counts_columns() {
        assert_eq!(ellipsize("abcdef", 6), "abcdef");
        assert_eq!(ellipsize("abcdefg", 6), "abcde…");
        assert_eq!(ellipsize("日本語日本語", 6), "日本…");
    }

    #[test]
    fn folder_uses_tilde() {
        assert_eq!(
            folder("/Users/me/Downloads/a.pdf", Some("/Users/me")),
            "~/Downloads"
        );
        assert_eq!(folder("/Users/me/a.pdf", Some("/Users/me")), "~");
        assert_eq!(folder("/tmp/a.pdf", Some("/Users/me")), "/tmp");
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(format_bytes(412 * 1024), "412 KB");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
        assert_eq!(plural(1, "word", "words"), "1 word");
    }
}
