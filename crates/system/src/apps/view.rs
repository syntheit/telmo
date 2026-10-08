//! The Force Quit view's state and keys. No drawing here.

use super::{AppRow, filter_rows};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use telmo_kit::input::TextInput;

/// How long "quitting…" waits for the app to go away.
const QUIT_PATIENCE: Duration = Duration::from_secs(5);

/// What the app should do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Quit { pid: i32, name: String, force: bool },
}

#[derive(Debug, Default)]
pub struct View {
    /// `None` until the first snapshot arrives.
    pub rows: Option<Vec<AppRow>>,
    pub error: Option<String>,
    pub selected: Option<i32>,
    pub filter: TextInput,
    /// The `/` field has the keyboard.
    pub searching: bool,
    /// The app a force quit is waiting on: pid and name.
    pub confirm: Option<(i32, String)>,
    /// Apps asked to quit, with the time of the request and whether forced.
    quitting: HashMap<i32, (Instant, bool)>,
}

impl View {
    pub fn set_rows(&mut self, result: Result<Vec<AppRow>, String>) {
        match result {
            Ok(rows) => {
                self.error = None;
                self.quitting
                    .retain(|pid, _| rows.iter().any(|row| row.pid == *pid));
                self.rows = Some(rows);
            }
            Err(message) => self.error = Some(message),
        }
    }

    /// The rows that match the filter.
    pub fn visible(&self) -> Vec<&AppRow> {
        let rows = self.rows.as_deref().unwrap_or(&[]);
        filter_rows(rows, &self.filter.value)
    }

    /// Index of the selection among the visible rows; the first row if the
    /// selected app is gone or filtered out.
    pub fn selected_index(&self) -> usize {
        let visible = self.visible();
        visible
            .iter()
            .position(|row| Some(row.pid) == self.selected)
            .unwrap_or(0)
            .min(visible.len().saturating_sub(1))
    }

    fn selected_row(&self) -> Option<&AppRow> {
        self.visible().get(self.selected_index()).copied()
    }

    pub fn select(&mut self, pid: i32) {
        self.selected = Some(pid);
    }

    fn step(&mut self, delta: isize) {
        let visible = self.visible();
        let last = visible.len().saturating_sub(1);
        let next = self.selected_index().saturating_add_signed(delta).min(last);
        self.selected = visible.get(next).map(|row| row.pid);
    }

    pub fn is_quitting(&self, pid: i32) -> bool {
        self.quitting.contains_key(&pid)
    }

    pub fn stop_quitting(&mut self, pid: i32) {
        self.quitting.remove(&pid);
    }

    /// Sentences for apps that ignored a quit for too long.
    pub fn tick(&mut self, now: Instant) -> Vec<String> {
        let late: Vec<i32> = self
            .quitting
            .iter()
            .filter(|(_, (since, _))| now.duration_since(*since) >= QUIT_PATIENCE)
            .map(|(pid, _)| *pid)
            .collect();
        let mut messages = Vec::new();
        for pid in late {
            let Some((_, forced)) = self.quitting.remove(&pid) else {
                continue;
            };
            let name = self.name_of(pid);
            messages.push(if forced {
                format!("{name} is still running. Try Activity Monitor.")
            } else {
                format!("{name} didn't quit. Press K to force it.")
            });
        }
        messages
    }

    fn name_of(&self, pid: i32) -> String {
        let rows = self.rows.as_deref().unwrap_or(&[]);
        rows.iter()
            .find(|row| row.pid == pid)
            .map_or_else(|| "The app".to_string(), |row| row.name.clone())
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.confirm.is_some() {
            return self.confirm_key(key);
        }
        if let Some(action) = self.move_key(key) {
            return action;
        }
        if self.searching {
            self.search_key(key)
        } else {
            self.list_key(key)
        }
    }

    /// Up and down work everywhere, also while typing a search.
    fn move_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Down => self.step(1),
            KeyCode::Up => self.step(-1),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            _ => return None,
        }
        Some(Action::None)
    }

    fn confirm_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => match self.confirm.take() {
                Some((pid, name)) => self.request_quit(pid, name, true),
                None => Action::None,
            },
            KeyCode::Esc | KeyCode::Char('q') => {
                self.confirm = None;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn search_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.filter.value.clear();
                self.searching = false;
            }
            KeyCode::Enter => self.searching = false,
            _ => {
                self.filter.handle(key);
            }
        }
        Action::None
    }

    fn list_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc if !self.filter.value.is_empty() => self.filter.value.clear(),
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Enter => return self.quit_selected(),
            KeyCode::Char('K') => {
                self.confirm = self.selected_row().map(|row| (row.pid, row.name.clone()));
            }
            _ => {}
        }
        Action::None
    }

    fn quit_selected(&mut self) -> Action {
        let Some((pid, name)) = self.selected_row().map(|row| (row.pid, row.name.clone())) else {
            return Action::None;
        };
        if self.is_quitting(pid) {
            return Action::None;
        }
        self.request_quit(pid, name, false)
    }

    fn request_quit(&mut self, pid: i32, name: String, force: bool) -> Action {
        self.quitting.insert(pid, (Instant::now(), force));
        Action::Quit { pid, name, force }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::mock;

    fn view() -> View {
        let mut view = View::default();
        view.set_rows(Ok(mock::rows()));
        view
    }

    fn press(view: &mut View, code: KeyCode) -> Action {
        view.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn enter_quits_the_selected_app_and_marks_it() {
        let mut view = view();
        press(&mut view, KeyCode::Down);
        let Action::Quit { pid, force, .. } = press(&mut view, KeyCode::Enter) else {
            panic!("expected a quit");
        };
        assert!(!force);
        assert!(view.is_quitting(pid));
        // A second Enter while it is quitting does nothing.
        assert_eq!(press(&mut view, KeyCode::Enter), Action::None);
    }

    #[test]
    fn force_quit_asks_first() {
        let mut view = view();
        assert_eq!(press(&mut view, KeyCode::Char('K')), Action::None);
        assert!(view.confirm.is_some());
        assert_eq!(press(&mut view, KeyCode::Esc), Action::None);
        assert!(view.confirm.is_none());
        press(&mut view, KeyCode::Char('K'));
        assert!(matches!(
            press(&mut view, KeyCode::Enter),
            Action::Quit { force: true, .. }
        ));
    }

    #[test]
    fn escape_clears_the_filter_before_closing() {
        let mut view = view();
        press(&mut view, KeyCode::Char('/'));
        press(&mut view, KeyCode::Char('z'));
        press(&mut view, KeyCode::Enter);
        assert_eq!(view.visible().len(), 1);
        assert_eq!(press(&mut view, KeyCode::Esc), Action::None);
        assert_eq!(view.visible().len(), 8);
        assert_eq!(press(&mut view, KeyCode::Esc), Action::Close);
    }

    #[test]
    fn typing_k_while_searching_does_not_open_the_dialog() {
        let mut view = view();
        press(&mut view, KeyCode::Char('/'));
        press(&mut view, KeyCode::Char('K'));
        assert!(view.confirm.is_none());
        assert_eq!(view.filter.value, "K");
    }

    #[test]
    fn selection_follows_the_app_when_the_list_changes() {
        let mut view = view();
        view.select(103);
        let mut rows = mock::rows();
        rows.retain(|row| row.pid != 101);
        view.set_rows(Ok(rows));
        assert_eq!(view.selected_row().map(|r| r.pid), Some(103));
        view.set_rows(Ok(Vec::new()));
        assert_eq!(view.selected_row(), None);
    }

    #[test]
    fn an_app_that_ignores_quit_is_reported_after_five_seconds() {
        let mut view = view();
        press(&mut view, KeyCode::Enter);
        assert!(view.tick(Instant::now()).is_empty());
        let later = Instant::now() + Duration::from_secs(6);
        let messages = view.tick(later);
        assert_eq!(messages, ["Telegram didn't quit. Press K to force it."]);
        assert!(view.tick(later).is_empty());
    }

    #[test]
    fn a_quitting_app_that_disappears_is_forgotten() {
        let mut view = view();
        press(&mut view, KeyCode::Enter);
        view.set_rows(Ok(Vec::new()));
        assert!(
            view.tick(Instant::now() + Duration::from_secs(9))
                .is_empty()
        );
    }
}
