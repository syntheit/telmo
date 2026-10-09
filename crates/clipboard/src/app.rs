//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event};
use crate::color::{Notation, Rgb};
use crate::model::{Entry, Kind, Snapshot};
use crate::{os, search};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use image::DynamicImage;
use ratatui_image::{picker::Picker, protocol::Protocol};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::mpsc::Sender,
    time::{SystemTime, UNIX_EPOCH},
};
use telmo_kit::{App as _, Flow, hits::Hits, input::TextInput, widgets::Toast};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    Delete(String),
    Help,
    ClearAll,
}

/// What a click on a drawn area does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    Row(String),
    Key(KeyCode),
    /// A hint inside a dialog.
    DialogKey(KeyCode),
}

pub struct App {
    pub snapshot: Snapshot,
    /// False until the backend has sent the first snapshot.
    pub loaded: bool,
    pub selected: Option<String>,
    /// `Some` while searching, even with nothing typed yet.
    pub search: Option<TextInput>,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub hits: Hits<Click>,
    /// Full text of long items, fetched on demand.
    pub texts: HashMap<String, String>,
    /// Loaded pictures; `None` means the load failed.
    pub images: HashMap<String, Option<DynamicImage>>,
    /// Pictures prepared for the terminal at the size they were drawn.
    pub protocols: RefCell<HashMap<(String, u16, u16), Protocol>>,
    /// The picture drawn by the frame being drawn, and by the frame before.
    pub drawn_image: RefCell<Option<(String, u16, u16)>>,
    last_image: Option<(String, u16, u16)>,
    pub picker: Picker,
    /// The item being put back on the clipboard.
    pub pending: Option<String>,
    pub tick: u64,
    pub home: Option<String>,
    /// Tests fix the clock so relative times don't drift.
    pub fixed_now: Option<u64>,
    notation: Option<(String, usize)>,
    requested: HashSet<String>,
    cmds: Sender<Cmd>,
}

impl App {
    pub fn new(cmds: Sender<Cmd>) -> Self {
        Self {
            snapshot: Snapshot::default(),
            loaded: false,
            selected: None,
            search: None,
            dialog: None,
            toast: None,
            hits: Hits::default(),
            texts: HashMap::new(),
            images: HashMap::new(),
            protocols: RefCell::new(HashMap::new()),
            drawn_image: RefCell::new(None),
            last_image: None,
            picker: Picker::halfblocks(),
            pending: None,
            tick: 0,
            home: std::env::var("HOME").ok(),
            fixed_now: None,
            notation: None,
            requested: HashSet::new(),
            cmds,
        }
    }

    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = picker;
    }

    pub fn now(&self) -> u64 {
        self.fixed_now.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
    }

    pub fn query(&self) -> &str {
        self.search.as_ref().map_or("", |s| s.value.trim())
    }

    /// What the list shows: pinned first, then the rest, both newest first.
    pub fn ordered(&self) -> Vec<&Entry> {
        let query = self.query();
        let all = &self.snapshot.entries;
        all.iter()
            .filter(|e| e.pinned)
            .chain(all.iter().filter(|e| !e.pinned))
            .filter(|e| search::matches(e, query))
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected.as_deref()?;
        self.snapshot.entries.iter().find(|e| e.id == id)
    }

    /// The whole text of an item, or what we have of it so far.
    pub fn full_text<'a>(&'a self, entry: &'a Entry) -> &'a str {
        self.texts.get(&entry.id).map_or(&entry.preview, |t| t)
    }

    /// The selected color item in the notation `step` presses on.
    fn color_in(&self, entry: &Entry, step: usize) -> Option<String> {
        let text = self.full_text(entry);
        let rgb = Rgb::parse(text)?;
        Some(rgb.format(Notation::of(text)?.after(step)))
    }

    /// The notations the next two `h` presses would copy.
    pub fn next_notations(&self, entry: &Entry) -> Option<[Notation; 2]> {
        let current = Notation::of(self.full_text(entry))?;
        let step = self.notation_step(entry) + 1;
        Some([current.after(step), current.after(step + 1)])
    }

    fn notation_step(&self, entry: &Entry) -> usize {
        match &self.notation {
            Some((id, step)) if *id == entry.id => *step,
            _ => 0,
        }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    fn toast_err(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::error(message));
    }

    // --- selection ---

    fn select_first(&mut self) {
        self.selected = self.ordered().first().map(|e| e.id.clone());
        self.ensure_loaded();
    }

    fn move_selection(&mut self, delta: isize) {
        let ids: Vec<String> = self.ordered().iter().map(|e| e.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = ids.iter().position(|id| Some(id) == self.selected.as_ref());
        let next = match at {
            Some(at) => at.saturating_add_signed(delta).min(ids.len() - 1),
            None => 0,
        };
        self.selected = Some(ids[next].clone());
        self.ensure_loaded();
    }

    /// Asks the backend for what the selected item needs to be shown.
    fn ensure_loaded(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let id = entry.id.clone();
        let wants_image = entry.kind == Kind::Image;
        let wants_text = !wants_image
            && entry.kind != Kind::File
            && entry.chars as usize > entry.preview.chars().count();
        if wants_image && !self.images.contains_key(&id) && self.requested.insert(id.clone()) {
            self.send(Cmd::LoadImage(id));
        } else if wants_text && !self.texts.contains_key(&id) && self.requested.insert(id.clone()) {
            self.send(Cmd::LoadText(id));
        }
    }

    fn new_snapshot(&mut self, snapshot: Snapshot) {
        let old_index = self
            .ordered()
            .iter()
            .position(|e| Some(&e.id) == self.selected.as_ref());
        self.snapshot = snapshot;
        let alive: HashSet<&str> = self
            .snapshot
            .entries
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        self.texts.retain(|id, _| alive.contains(id.as_str()));
        self.images.retain(|id, _| alive.contains(id.as_str()));
        self.requested.retain(|id| alive.contains(id.as_str()));
        self.protocols
            .borrow_mut()
            .retain(|(id, _, _), _| alive.contains(id.as_str()));
        if !self.loaded {
            self.loaded = true;
            self.selected = self.snapshot.entries.first().map(|e| e.id.clone());
        } else if self.selected_entry().is_none() {
            let ordered = self.ordered();
            let at = old_index.unwrap_or(0).min(ordered.len().saturating_sub(1));
            self.selected = ordered.get(at).map(|e| e.id.clone());
        }
        self.ensure_loaded();
    }

    // --- actions ---

    fn copy_selected(&mut self) {
        let Some(id) = self.selected_entry().map(|e| e.id.clone()) else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        self.pending = Some(id.clone());
        self.send(Cmd::Copy {
            id,
            text: None,
            close: true,
        });
    }

    fn copy_notation(&mut self) {
        let Some(entry) = self.selected_entry().filter(|e| e.kind == Kind::Color) else {
            return;
        };
        let (id, step) = (entry.id.clone(), self.notation_step(entry) + 1);
        let Some(text) = self.color_in(entry, step) else {
            return;
        };
        self.notation = Some((id.clone(), step));
        self.send(Cmd::Copy {
            id,
            text: Some(text),
            close: false,
        });
    }

    fn toggle_pin(&mut self) {
        if let Some(entry) = self.selected_entry() {
            let cmd = Cmd::Pin {
                id: entry.id.clone(),
                pinned: !entry.pinned,
            };
            self.send(cmd);
        }
    }

    fn open_selected(&mut self) -> Flow {
        let Some(entry) = self.selected_entry() else {
            return Flow::Continue;
        };
        let result = match entry.kind {
            Kind::Link => telmo_kit::os::open(&os::link_target(self.full_text(entry))),
            Kind::File => match entry.files.first() {
                Some(path) => telmo_kit::os::open(path),
                None => return Flow::Continue,
            },
            _ => return Flow::Continue,
        };
        self.after_launch(result)
    }

    fn reveal_selected(&mut self) -> Flow {
        let Some(path) = self
            .selected_entry()
            .filter(|e| e.kind == Kind::File)
            .and_then(|e| e.files.first().cloned())
        else {
            return Flow::Continue;
        };
        self.after_launch(os::reveal(&path))
    }

    fn after_launch(&mut self, result: Result<(), String>) -> Flow {
        match result {
            Ok(()) => Flow::Quit,
            Err(message) => {
                self.toast_err(message);
                Flow::Continue
            }
        }
    }

    // --- keys ---

    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Flow {
        match (dialog, key.code) {
            (Dialog::Delete(id), KeyCode::Enter) => {
                self.send(Cmd::Delete(id));
                self.dialog = None;
            }
            (Dialog::ClearAll, KeyCode::Enter) => {
                self.send(Cmd::Clear);
                self.dialog = None;
            }
            (Dialog::Help, KeyCode::Char('c')) => self.dialog = Some(Dialog::ClearAll),
            (Dialog::Help, KeyCode::Char('?' | 'q')) | (_, KeyCode::Esc) => self.dialog = None,
            _ => {}
        }
        Flow::Continue
    }

    fn search_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc => {
                self.search = None;
                self.reselect();
            }
            KeyCode::Enter => self.copy_selected(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            _ => {
                let edited = self.search.as_mut().is_some_and(|s| s.handle(key));
                if edited {
                    self.select_first();
                }
            }
        }
        Flow::Continue
    }

    /// After leaving search the selected item stays if it is still listed.
    fn reselect(&mut self) {
        if self.selected_entry().is_none() {
            self.select_first();
        }
    }

    fn start_search(&mut self) {
        self.search = Some(TextInput::default());
    }

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Enter => self.copy_selected(),
            KeyCode::Char('p') => self.toggle_pin(),
            KeyCode::Char('x') => {
                if let Some(entry) = self.selected_entry() {
                    self.dialog = Some(Dialog::Delete(entry.id.clone()));
                }
            }
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            KeyCode::Char('o') => return self.open_selected(),
            KeyCode::Char('r') => return self.reveal_selected(),
            KeyCode::Char('h') => self.copy_notation(),
            _ => {}
        }
        Flow::Continue
    }

    fn click(&mut self, click: Click) -> Flow {
        match click {
            Click::Row(id) => {
                self.selected = Some(id);
                self.ensure_loaded();
                Flow::Continue
            }
            Click::Key(code) if self.dialog.is_none() => self.key(KeyEvent::from(code)),
            Click::DialogKey(code) => self.key(KeyEvent::from(code)),
            Click::Key(_) => Flow::Continue,
        }
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    /// Telmo.app's terminal keeps an old picture under the new one until the screen is cleared.
    fn take_clear(&mut self) -> bool {
        let now = self.drawn_image.borrow().clone();
        let stale = self.last_image.is_some() && self.last_image != now;
        self.last_image = now;
        stale
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if key.modifiers.contains(KeyModifiers::CONTROL) && self.search.is_none() {
            return Flow::Continue;
        }
        if let Some(dialog) = self.dialog.clone() {
            return self.dialog_key(dialog, key);
        }
        if self.search.is_some() {
            self.search_key(key)
        } else {
            self.main_key(key)
        }
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                match self.hits.at(event.column, event.row) {
                    Some(click) => self.click(click),
                    None => Flow::Continue,
                }
            }
            MouseEventKind::ScrollUp if self.dialog.is_none() => {
                self.move_selection(-1);
                Flow::Continue
            }
            MouseEventKind::ScrollDown if self.dialog.is_none() => {
                self.move_selection(1);
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Snapshot(snapshot) => self.new_snapshot(snapshot),
            Event::Text { id, text } => {
                self.texts.insert(id, text);
            }
            Event::Image { id, image } => {
                self.images.insert(id, image);
            }
            Event::Failed(message) => self.toast_err(message),
            Event::Copied { close, result } => {
                self.pending = None;
                match result {
                    Ok(None) if close => return Flow::Quit,
                    Ok(None) => {}
                    Ok(Some(message)) => self.toast = Some(Toast::ok(message)),
                    Err(message) => self.toast_err(message),
                }
            }
        }
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        self.tick += 1;
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.pending.is_some() || self.toast.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock;
    use std::sync::mpsc::{Receiver, channel};

    const NOW: u64 = 1_790_000_000;

    fn app() -> (App, Receiver<Cmd>) {
        let (tx, rx) = channel();
        let mut app = App::new(tx);
        app.fixed_now = Some(NOW);
        app.event(Event::Snapshot(Snapshot {
            entries: mock::entries(NOW),
            max_items: 50,
            max_days: 30,
        }));
        (app, rx)
    }

    fn press(app: &mut App, code: KeyCode) -> Flow {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn sent(rx: &Receiver<Cmd>) -> Vec<Cmd> {
        rx.try_iter().collect()
    }

    #[test]
    fn a_replaced_or_removed_picture_clears_the_screen_once() {
        use telmo_kit::App as _;
        let (mut app, _rx) = app();
        let drew = |app: &mut App, image: Option<&str>| {
            app.drawn_image
                .replace(image.map(|id| (id.to_string(), 10, 5)));
            app.take_clear()
        };
        assert!(!drew(&mut app, Some("a")), "first picture");
        assert!(!drew(&mut app, Some("a")), "same picture");
        assert!(drew(&mut app, Some("b")), "another picture");
        assert!(drew(&mut app, None), "picture gone");
        assert!(!drew(&mut app, None), "still none");
    }

    #[test]
    fn newest_is_selected_and_moving_follows_the_list_order() {
        let (mut app, _rx) = app();
        assert_eq!(app.selected.as_deref(), Some("sudo"));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected.as_deref(), Some("shot"));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        // Pinned items come first in the list.
        assert_eq!(app.selected.as_deref(), Some("phone"));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected.as_deref(), Some("ssh"));
    }

    #[test]
    fn enter_copies_and_closes_when_the_backend_confirms() {
        let (mut app, rx) = app();
        assert_eq!(press(&mut app, KeyCode::Enter), Flow::Continue);
        assert!(
            matches!(&sent(&rx)[..], [Cmd::Copy { id, text: None, close: true }] if id == "sudo")
        );
        assert_eq!(app.pending.as_deref(), Some("sudo"));
        let done = Event::Copied {
            close: true,
            result: Ok(None),
        };
        assert_eq!(app.event(done), Flow::Quit);
    }

    #[test]
    fn copy_failures_and_mock_copies_stay_open_with_a_toast() {
        let (mut app, _rx) = app();
        press(&mut app, KeyCode::Enter);
        let failed = Event::Copied {
            close: true,
            result: Err("wl-copy failed.".into()),
        };
        assert_eq!(app.event(failed), Flow::Continue);
        assert!(app.pending.is_none());
        assert!(!app.toast.as_ref().unwrap().ok);
        let mocked = Event::Copied {
            close: true,
            result: Ok(Some("Would copy x".into())),
        };
        assert_eq!(app.event(mocked), Flow::Continue);
        assert!(app.toast.as_ref().unwrap().ok);
    }

    #[test]
    fn delete_asks_first() {
        let (mut app, rx) = app();
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.dialog, Some(Dialog::Delete("sudo".into())));
        press(&mut app, KeyCode::Esc);
        assert!(app.dialog.is_none());
        assert!(sent(&rx).is_empty());
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(&sent(&rx)[..], [Cmd::Delete(id)] if id == "sudo"));
    }

    #[test]
    fn deleting_moves_the_selection_to_a_neighbor() {
        let (mut app, _rx) = app();
        let mut snapshot = app.snapshot.clone();
        snapshot.entries.retain(|e| e.id != "sudo");
        app.event(Event::Snapshot(snapshot));
        assert_eq!(app.selected.as_deref(), Some("shot"));
    }

    #[test]
    fn pin_toggles() {
        let (mut app, rx) = app();
        press(&mut app, KeyCode::Char('p'));
        assert!(matches!(&sent(&rx)[..], [Cmd::Pin { pinned: true, .. }]));
        app.selected = Some("ssh".into());
        press(&mut app, KeyCode::Char('p'));
        assert!(matches!(&sent(&rx)[..], [Cmd::Pin { pinned: false, .. }]));
    }

    #[test]
    fn escape_clears_the_search_before_closing() {
        let (mut app, _rx) = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "hunter".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.ordered().len(), 1);
        assert_eq!(app.selected.as_deref(), Some("secret"));
        assert_eq!(press(&mut app, KeyCode::Esc), Flow::Continue);
        assert!(app.search.is_none());
        assert_eq!(
            app.selected.as_deref(),
            Some("secret"),
            "the item stays selected"
        );
        assert_eq!(press(&mut app, KeyCode::Esc), Flow::Quit);
    }

    #[test]
    fn typing_in_search_does_not_trigger_keys() {
        let (mut app, rx) = app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('p'));
        assert!(app.dialog.is_none());
        assert!(sent(&rx).is_empty());
        assert_eq!(app.search.as_ref().unwrap().value, "xqp");
    }

    #[test]
    fn color_notation_cycles() {
        let (mut app, rx) = app();
        app.selected = Some("color".into());
        let copied = |rx: &Receiver<Cmd>| match &sent(rx)[..] {
            [
                Cmd::Copy {
                    text: Some(t),
                    close: false,
                    ..
                },
            ] => t.clone(),
            other => panic!("unexpected {other:?}"),
        };
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(copied(&rx), "rgb(122, 162, 247)");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(copied(&rx), "hsl(221, 89%, 72%)");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(copied(&rx), "#7aa2f7");
        // Other kinds ignore it.
        app.selected = Some("sudo".into());
        press(&mut app, KeyCode::Char('h'));
        assert!(sent(&rx).is_empty());
    }

    #[test]
    fn selecting_a_picture_asks_for_it_once() {
        let (mut app, rx) = app();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Down);
        let loads: Vec<_> = sent(&rx)
            .into_iter()
            .filter(|c| matches!(c, Cmd::LoadImage(_)))
            .collect();
        assert_eq!(loads.len(), 1);
    }

    #[test]
    fn clearing_everything_needs_the_help_and_a_confirm() {
        let (mut app, rx) = app();
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.dialog, Some(Dialog::ClearAll));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(&sent(&rx)[..], [Cmd::Clear]));
    }

    #[test]
    fn clicking_a_row_selects_and_scrolling_moves() {
        let (mut app, _rx) = app();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(108, 26)).unwrap();
        terminal.draw(|f| crate::ui::draw(&app, f)).unwrap();
        // Row of "Screenshot" in the sidebar.
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 8,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(click);
        assert_eq!(app.selected.as_deref(), Some("shot"));
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..click
        };
        app.mouse(scroll);
        assert_eq!(app.selected.as_deref(), Some("link"));
    }

    #[test]
    fn long_text_is_fetched_when_selected() {
        let (mut app, rx) = app();
        let mut snapshot = app.snapshot.clone();
        snapshot.entries[3].chars = 5000;
        app.event(Event::Snapshot(snapshot));
        app.selected = Some("sudo".into());
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert!(
            sent(&rx)
                .iter()
                .any(|c| matches!(c, Cmd::LoadText(id) if id == "color"))
        );
    }
}
