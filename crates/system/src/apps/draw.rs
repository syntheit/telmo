//! Drawing the Force Quit view: a pure function of the app state.

use super::view::View;
use super::{AppRow, format_memory};
use crate::app::{App, Click};
use crate::ui::{open_dialog, padded_hint};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const MEMORY_WIDTH: usize = 9;
const CPU_WIDTH: usize = 5;
const NOTE_WIDTH: usize = 15;
/// The selection marker column that `widgets::rows` adds to every row.
const MARKER_WIDTH: usize = 3;

pub fn draw(app: &App, view: &View, frame: &mut Frame) {
    let area = frame.area();
    // One blank row above, then a toast row and the key bar below the panel.
    let panel = widgets::dialog(
        frame,
        "Force quit",
        area.width,
        area.height.saturating_sub(3),
    );
    draw_sort_label(frame, panel);
    draw_body(app, view, frame, panel);

    if area.height > 1 {
        let y = area.y + area.height - 2;
        if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
            toast.render(
                frame,
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }
    }
    let keys: &[(&str, &str)] = if view.searching {
        &[("↵", "done"), ("esc", "clear")]
    } else {
        &[
            ("↵", "quit"),
            ("K", "force quit"),
            ("/", "search"),
            ("esc", "back"),
        ]
    };
    let row = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };
    widgets::keys(frame, row, keys);

    if let Some((_, name)) = &view.confirm {
        draw_confirm(app, frame, name);
    }
}

/// "by memory" on the right of the top border.
fn draw_sort_label(frame: &mut Frame, panel: Rect) {
    let Some(y) = panel.y.checked_sub(1) else {
        return;
    };
    let label = Line::from(vec![
        Span::styled(" by memory ", theme::dim()),
        Span::styled("─", theme::accent()),
    ]);
    frame.render_widget(
        label.right_aligned(),
        Rect {
            y,
            height: 1,
            ..panel
        },
    );
}

fn draw_body(app: &App, view: &View, frame: &mut Frame, panel: Rect) {
    let width = panel.width as usize;
    let header = Rect { height: 1, ..panel };
    let show_search = view.searching || !view.filter.value.is_empty();
    let search_height = u16::from(show_search);
    let list = Rect {
        y: panel.y + 1,
        height: panel.height.saturating_sub(1 + search_height),
        ..panel
    };
    let columns = Columns::new(width);
    frame.render_widget(columns.header(), header);

    let visible = view.visible();
    if visible.is_empty() {
        let message = empty_message(view);
        widgets::text(frame, list, vec![Line::styled(message, theme::dim())]);
    } else {
        let lines = visible.iter().map(|row| columns.row(view, row)).collect();
        let drawn = widgets::rows(frame, list, lines, Some(view.selected_index()));
        for (index, rect) in drawn {
            app.hits.add(rect, Click::AppRow(visible[index].pid));
        }
    }
    if show_search {
        let row = Rect {
            y: panel.y + panel.height - 1,
            height: 1,
            ..panel
        };
        draw_search(view, frame, row);
    }
}

fn empty_message(view: &View) -> String {
    if let Some(error) = &view.error {
        return format!("  {error}");
    }
    match (&view.rows, view.filter.value.trim()) {
        (None, _) => "  Looking for apps…".into(),
        (Some(_), "") => "  No apps are open.".into(),
        (Some(_), query) => format!("  No app is called \"{query}\"."),
    }
}

fn draw_search(view: &View, frame: &mut Frame, row: Rect) {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled("/", theme::accent()),
        Span::raw(" "),
    ];
    spans.extend(
        view.filter
            .spans((row.width as usize).saturating_sub(5), view.searching),
    );
    frame.render_widget(Line::from(spans), row);
}

/// Column widths for one panel width.
struct Columns {
    name: usize,
}

impl Columns {
    fn new(panel_width: usize) -> Self {
        let fixed = MARKER_WIDTH + MEMORY_WIDTH + CPU_WIDTH + NOTE_WIDTH + 3 * 2;
        Self {
            name: panel_width.saturating_sub(fixed),
        }
    }

    fn header(&self) -> Line<'static> {
        let text = format!(
            "{}{}  {:>MEMORY_WIDTH$}  {:>CPU_WIDTH$}",
            " ".repeat(MARKER_WIDTH),
            widgets::fit("App", self.name),
            "Memory",
            "CPU",
        );
        Line::styled(text, theme::dim())
    }

    /// A row without the marker; `widgets::rows` adds that.
    fn row(&self, view: &View, row: &AppRow) -> Line<'static> {
        let cpu = row.cpu.map_or("—".to_string(), |cpu| format!("{cpu:.0}%"));
        let (note, style) = note(view, row);
        Line::from(vec![
            Span::styled(widgets::fit(&row.name, self.name), theme::text()),
            Span::raw("  "),
            Span::styled(
                format!("{:>MEMORY_WIDTH$}", format_memory(row.memory)),
                theme::text(),
            ),
            Span::raw("  "),
            Span::styled(format!("{cpu:>CPU_WIDTH$}"), theme::dim()),
            Span::raw("  "),
            Span::styled(widgets::fit(&note, NOTE_WIDTH), style),
        ])
    }
}

fn note(view: &View, row: &AppRow) -> (String, ratatui::style::Style) {
    if view.is_quitting(row.pid) {
        ("quitting…".into(), theme::warn())
    } else if row.unresponsive {
        ("not responding".into(), theme::err())
    } else if row.windows > 1 {
        (format!("{} windows", row.windows), theme::dim())
    } else {
        (String::new(), theme::dim())
    }
}

fn draw_confirm(app: &App, frame: &mut Frame, name: &str) {
    let verbs = [("↵", "force quit"), ("esc", "cancel")];
    let inner = open_dialog(app, frame, &format!("Force quit {name}?"), 46, 6);
    let lines = vec![
        Line::raw(""),
        Line::styled("  Unsaved changes in it are lost.", theme::dim()),
        Line::raw(""),
        padded_hint(&verbs),
    ];
    widgets::text(frame, inner, lines);
    let hint = Rect {
        y: inner.y + 3,
        height: 1,
        x: inner.x + 1,
        width: inner.width.saturating_sub(1),
    };
    if let Some(area) = widgets::hint_areas(hint, &verbs).first() {
        app.hits.add(*area, Click::Confirm);
    }
}

#[cfg(test)]
mod tests {
    use crate::actions::Event;
    use crate::app::App;
    use crate::apps::mock;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use telmo_kit::App as _;
    use tokio::sync::mpsc::unbounded_channel;

    fn open_view() -> App {
        let (tx, _rx) = unbounded_channel();
        let mut app = App::for_test(tx, (90, 22));
        press(&mut app, KeyCode::Char('k'));
        app.event(Event::Apps(Ok(mock::rows())));
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn render(app: &App) -> String {
        telmo_kit::test::render(90, 22, |f| crate::ui::draw(app, f))
    }

    #[test]
    fn the_list() {
        insta::assert_snapshot!(render(&open_view()));
    }

    #[test]
    fn the_list_before_the_first_snapshot() {
        let (tx, _rx) = unbounded_channel();
        let mut app = App::for_test(tx, (90, 22));
        press(&mut app, KeyCode::Char('k'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn filter_active() {
        let mut app = open_view();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('s'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn force_quit_confirm() {
        let mut app = open_view();
        press(&mut app, KeyCode::Char('K'));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn quitting_row() {
        let mut app = open_view();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(render(&app));
    }
}
