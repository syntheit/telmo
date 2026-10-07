//! The "What's playing?" dialog.

use super::{dialog_hits, hint_hits, padded};
use crate::app::App;
use crate::model::Source;
use crate::song::{Found, Listen, Phase};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use telmo_kit::{theme, widgets};

const WIDTH: u16 = 72;
/// Rows inside the frame. Every state uses the same size, so the dialog
/// doesn't jump while it goes from listening to the result.
const ROWS: u16 = 11;
const GAUGE_WIDTH: usize = 40;
const RECENT: usize = 3;
const INDENT: &str = "  ";

pub fn draw(app: &App, frame: &mut Frame, listen: &Listen) {
    let inner = widgets::dialog(frame, "What's playing?", WIDTH, ROWS + 2);
    dialog_hits(app, frame, inner);
    let mut lines = vec![Line::raw("")];
    lines.extend(match &listen.phase {
        Phase::Listening | Phase::Recognizing => progress(listen),
        Phase::Found(found) => result(app, found),
        Phase::NoMatch => vec![text("No match. Try again closer to the speaker.")],
        Phase::Failed(message) => wrap(message, WIDTH as usize - 6)
            .into_iter()
            .map(|row| text(&row))
            .collect(),
    });
    widgets::text(frame, inner, lines);

    if let Some(note) = &listen.note {
        let style = if note.error {
            theme::err()
        } else {
            theme::ok()
        };
        let row = Rect {
            y: inner.y + ROWS - 2,
            height: 1,
            ..inner
        };
        frame.render_widget(Line::styled(format!("{INDENT}{}", note.text), style), row);
    }
    let bindings = bindings(listen);
    let row = Rect {
        y: inner.y + ROWS - 1,
        height: 1,
        ..inner
    };
    frame.render_widget(padded(widgets::hint(&bindings)), row);
    hint_hits(app, row, &bindings);
}

fn text(message: &str) -> Line<'static> {
    Line::styled(format!("{INDENT}{message}"), theme::text())
}

fn dim(message: &str) -> Line<'static> {
    Line::styled(format!("{INDENT}{message}"), theme::dim())
}

fn source_line(source: Source) -> &'static str {
    match source {
        Source::Desktop => "Listening to desktop audio",
        Source::Mic => "Listening through the microphone",
    }
}

fn progress(listen: &Listen) -> Vec<Line<'static>> {
    let headline = if listen.phase == Phase::Recognizing {
        text("Recognizing…")
    } else if listen.retrying() {
        Line::from(vec![
            Span::styled(
                format!("{INDENT}{}", source_line(listen.source)),
                theme::text(),
            ),
            Span::styled("  trying once more", theme::dim()),
        ])
    } else {
        text(source_line(listen.source))
    };
    let heard = format!(
        "  {} / {} s",
        listen.heard_seconds().min(listen.target_seconds() as f32) as usize,
        listen.target_seconds()
    );
    let mut bar = vec![Span::raw(INDENT)];
    bar.extend(widgets::gauge(
        listen.progress(),
        GAUGE_WIDTH,
        theme::accent(),
    ));
    bar.push(Span::styled(heard, theme::dim()));
    let mut meter = vec![Span::raw(INDENT)];
    meter.extend(widgets::gauge(listen.level, GAUGE_WIDTH, theme::ok()));
    meter.push(Span::styled("  level", theme::dim()));
    vec![headline, Line::raw(""), Line::from(bar), Line::from(meter)]
}

fn result(app: &App, found: &Found) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(format!("{INDENT}{}", found.title), theme::bold()),
        text(&found.artist),
    ];
    let details = found.details();
    lines.push(if details.is_empty() {
        Line::raw("")
    } else {
        dim(&details)
    });
    let before: Vec<&Found> = app
        .history
        .iter()
        .filter(|s| *s != found)
        .take(RECENT)
        .collect();
    if !before.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{INDENT}Recent"), theme::faint()));
        lines.extend(before.iter().map(|s| dim(&s.label())));
    }
    lines
}

fn bindings(listen: &Listen) -> Vec<(&'static str, &'static str)> {
    let other = match listen.source {
        Source::Desktop => ("m", "use microphone"),
        Source::Mic => ("m", "use desktop audio"),
    };
    match &listen.phase {
        Phase::Listening | Phase::Recognizing => vec![other, ("esc", "cancel")],
        Phase::Found(found) => {
            let mut keys = vec![("c", if copied(listen) { "copied" } else { "copy" })];
            if found.shazam_url.is_some() {
                keys.push(("o", "Shazam"));
            }
            if found.apple_music_url.is_some() {
                keys.push(("a", "Apple Music"));
            }
            keys.extend([("s", "Spotify"), ("r", "again"), ("esc", "close")]);
            keys
        }
        Phase::NoMatch | Phase::Failed(_) => vec![("r", "retry"), other, ("esc", "close")],
    }
}

fn copied(listen: &Listen) -> bool {
    listen.note.as_ref().is_some_and(|n| !n.error)
}

/// Greedy word wrap.
fn wrap(message: &str, width: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for word in message.split_whitespace() {
        match rows.last_mut() {
            Some(row) if row.chars().count() + 1 + word.chars().count() <= width => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_string()),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_word_boundaries() {
        let rows = wrap("one two three four", 9);
        assert_eq!(rows, ["one two", "three", "four"]);
    }
}
