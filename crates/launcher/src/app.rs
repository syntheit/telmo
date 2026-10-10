//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event};
use crate::commands::{self, Action, Command, Env, Power};
use crate::config::Config;
use crate::history::History;
use crate::math::{Calc, Math};
use crate::mode::Mode;
use crate::model::AppEntry;
use crate::rank::{self, Ranker};
use crate::web::{self, Engine};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use image::DynamicImage;
use ratatui_image::{picker::Picker, protocol::Protocol};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    sync::mpsc::Sender,
    time::{SystemTime, UNIX_EPOCH},
};
use telmo_kit::{Flow, hits::Hits, input::TextInput, widgets::Toast};

/// Lines the list can hold at most.
pub const MAX_LINES: usize = 9;
/// Lines the frame spends on everything but the list: border, query,
/// separator, border and the key bar.
pub const CHROME: u16 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    App {
        app: usize,
        hits: Vec<u32>,
    },
    Math(Math),
    Command {
        command: usize,
        hits: Vec<u32>,
    },
    /// A search on the engine at this index.
    Search {
        engine: usize,
    },
    Timer {
        duration: String,
        name: Option<String>,
    },
}

impl Row {
    /// Lines the row takes: a result with related values takes two.
    pub fn lines(&self) -> usize {
        match self {
            Row::Math(m) if !m.related.is_empty() => 2,
            _ => 1,
        }
    }
}

/// What a click on a drawn area does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    Row(usize),
}

pub struct App {
    pub apps: Vec<AppEntry>,
    pub running: HashSet<String>,
    pub config: Config,
    pub history: History,
    pub commands: Vec<Command>,
    pub engines: Vec<Engine>,
    pub input: TextInput,
    pub rows: Vec<Row>,
    pub selected: usize,
    /// Asking before a restart, shutdown or logout.
    pub dialog: Option<Power>,
    pub toast: Option<Toast>,
    pub host: String,
    pub clock: bool,
    pub hits: Hits<Click>,
    /// Loaded icons; `None` means the app has none we can draw.
    pub images: HashMap<String, Option<DynamicImage>>,
    /// Icons prepared for the terminal.
    pub protocols: RefCell<HashMap<String, Protocol>>,
    /// The icons the frame being drawn put on screen, and the frame before.
    pub drawn_images: RefCell<Vec<(u16, u16, String)>>,
    last_images: Vec<(u16, u16, String)>,
    pub picker: Picker,
    /// The size seen by the last draw; the list fits it.
    pub area: Cell<(u16, u16)>,
    /// Waiting for the backend to finish what ↵ asked.
    pub pending: bool,
    /// Tests fix the clock so frecency doesn't drift.
    pub fixed_now: Option<u64>,
    calc: Calc,
    ranker: Ranker,
    requested: HashSet<String>,
    cmds: Sender<Cmd>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An app's letter key: given for its name, or for one of its other names.
fn key_in(config: &Config, app: &AppEntry) -> Option<char> {
    config
        .key_of(&app.name)
        .or_else(|| app.aliases.iter().find_map(|alias| config.key_of(alias)))
}

impl App {
    pub fn new(
        cmds: Sender<Cmd>,
        config: Config,
        history: History,
        apps: Vec<AppEntry>,
        env: Env,
        host: String,
    ) -> Self {
        let size = crossterm::terminal::size().unwrap_or((66, 14));
        let engines = web::engines(&config);
        let mut app = Self {
            apps,
            running: HashSet::new(),
            config,
            history,
            commands: commands::all(env),
            engines,
            input: TextInput::default(),
            rows: Vec::new(),
            selected: 0,
            dialog: None,
            toast: None,
            host,
            clock: env.clock,
            hits: Hits::default(),
            images: HashMap::new(),
            protocols: RefCell::new(HashMap::new()),
            drawn_images: RefCell::new(Vec::new()),
            last_images: Vec::new(),
            picker: Picker::halfblocks(),
            area: Cell::new(size),
            pending: false,
            fixed_now: None,
            calc: Calc::new(),
            ranker: Ranker::new(),
            requested: HashSet::new(),
            cmds,
        };
        app.refresh(false);
        app
    }

    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = picker;
    }

    pub fn query(&self) -> &str {
        &self.input.value
    }

    /// Lines the list may use in the current window.
    fn capacity(&self) -> usize {
        usize::from(self.area.get().1.saturating_sub(CHROME)).clamp(1, MAX_LINES)
    }

    /// The letter shown for an app: `fn T`.
    pub fn hotkey_label(&self, app: &AppEntry) -> Option<String> {
        key_in(&self.config, app).map(|k| self.config.hotkey_text(k))
    }

    /// The word on the right of the query line.
    pub fn mode_word(&self) -> &'static str {
        match Mode::parse(&self.input.value) {
            Mode::Recent => "recent",
            Mode::Apps(_) => "apps",
            Mode::Math { .. } if matches!(self.rows.first(), Some(Row::Math(_))) => "math",
            Mode::Math { .. } => "apps",
            Mode::Commands(_) => "commands",
            Mode::Web(_) => "web",
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Identifies a row across recomputations, so the selection can stay on it.
    fn row_key(&self, row: &Row) -> String {
        match row {
            Row::App { app, .. } => format!("app:{}", self.apps[*app].id),
            Row::Math(_) => "math".into(),
            Row::Command { command, .. } => format!("cmd:{}", self.commands[*command].label),
            Row::Search { engine } => format!("web:{}", self.engines[*engine].key),
            Row::Timer { .. } => "timer".into(),
        }
    }

    fn app_rows(&mut self, query: &str, limit: usize) -> Vec<Row> {
        let config = &self.config;
        if query.is_empty() {
            return rank::recent(&self.apps, &self.history, |a| key_in(config, a), limit)
                .into_iter()
                .map(|app| Row::App {
                    app,
                    hits: Vec::new(),
                })
                .collect();
        }
        let now = self.fixed_now.unwrap_or_else(unix_now);
        self.ranker
            .apps(
                &self.apps,
                query,
                &self.history,
                |a| key_in(config, a),
                now,
                limit,
            )
            .into_iter()
            .map(|(app, hits)| Row::App { app, hits })
            .collect()
    }

    /// Recomputes the list for the query. `keep` leaves the selection on the
    /// same row when it is still there (a background update), else it goes
    /// back to the top (the query changed).
    pub fn refresh(&mut self, keep: bool) {
        let previous = keep
            .then(|| self.selected_row().map(|r| self.row_key(r)))
            .flatten();
        let query = self.input.value.clone();
        let lines = self.capacity();
        let mut rows = Vec::new();
        match Mode::parse(&query) {
            Mode::Recent => rows = self.app_rows("", lines),
            Mode::Apps(q) => rows = self.app_rows(q, lines),
            Mode::Math { expr, apps } => {
                if let Some(math) = self.calc.eval(expr, !apps) {
                    rows.push(Row::Math(math));
                }
                if apps {
                    rows.extend(self.app_rows(expr, lines));
                }
            }
            Mode::Commands(q) => {
                if let Some((duration, name)) = commands::timer_spec(q) {
                    rows.push(Row::Timer { duration, name });
                }
                rows.extend(
                    commands::filter(&self.commands, q, &mut self.ranker)
                        .into_iter()
                        .map(|(command, hits)| Row::Command { command, hits }),
                );
            }
            Mode::Web(rest) => {
                let (named, _) = web::split(rest, &self.engines);
                let first = named.unwrap_or(0);
                rows.push(Row::Search { engine: first });
                rows.extend(
                    (0..self.engines.len())
                        .filter(|e| *e != first)
                        .map(|engine| Row::Search { engine }),
                );
            }
        }
        // Keep what fits.
        let mut used = 0;
        rows.retain(|row| {
            used += row.lines();
            used <= lines
        });
        self.rows = rows;
        self.selected = previous
            .and_then(|key| self.rows.iter().position(|r| self.row_key(r) == key))
            .unwrap_or(0);
        self.request_icons();
    }

    /// Asks the backend for the icons the visible rows need.
    fn request_icons(&mut self) {
        let wanted: Vec<AppEntry> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::App { app, .. } => Some(self.apps[*app].clone()),
                _ => None,
            })
            .filter(|app| !self.images.contains_key(&app.id))
            .collect();
        for app in wanted {
            if self.requested.insert(app.id.clone()) {
                let _ = self.cmds.send(Cmd::Icon(app));
            }
        }
    }

    pub fn running(&self, app: &AppEntry) -> bool {
        self.running.contains(&app.id)
    }

    fn move_selection(&mut self, step: isize) {
        let n = self.rows.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + step).rem_euclid(n) as usize;
        }
    }

    fn send(&mut self, cmd: Cmd) {
        if self.cmds.send(cmd).is_ok() {
            self.pending = true;
        }
    }

    fn error(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::error(message));
    }

    /// What was typed to find an app, for learning.
    fn typed_for_app(&self) -> String {
        match Mode::parse(&self.input.value) {
            Mode::Apps(q) => q.to_string(),
            Mode::Math { expr, apps: true } => expr.to_string(),
            _ => String::new(),
        }
    }

    /// ↵ on the selected row (`with_unit` is tab on a result).
    fn activate(&mut self, with_unit: bool) -> Flow {
        if self.pending {
            return Flow::Continue;
        }
        let Some(row) = self.selected_row().cloned() else {
            return Flow::Continue;
        };
        match row {
            Row::App { app, .. } => {
                let query = self.typed_for_app();
                let app = self.apps[app].clone();
                self.send(Cmd::Open { app, query });
            }
            Row::Math(math) => {
                let text = if with_unit {
                    math.with_unit
                } else {
                    math.plain
                };
                self.send(Cmd::Copy(text));
            }
            Row::Command { command, .. } => match self.commands[command].action.clone() {
                Action::Power(power) if power.confirms() => self.dialog = Some(power),
                action => self.send(Cmd::Run(action)),
            },
            Row::Search { engine } => {
                let Mode::Web(rest) = Mode::parse(&self.input.value) else {
                    return Flow::Continue;
                };
                let (_, words) = web::split(rest, &self.engines);
                match web::url_for(&self.engines[engine], words) {
                    Some(url) => self.send(Cmd::OpenUrl(url)),
                    None => self.error("Type what to search for first."),
                }
            }
            Row::Timer { duration, name } => {
                if self.clock {
                    self.send(Cmd::Run(Action::Timer { duration, name }));
                } else {
                    self.error("Clock isn't installed.");
                }
            }
        }
        Flow::Continue
    }

    fn dialog_key(&mut self, power: Power, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Enter => {
                self.dialog = None;
                self.send(Cmd::Run(Action::Power(power)));
            }
            KeyCode::Esc | KeyCode::Char('n') => self.dialog = None,
            _ => {}
        }
        Flow::Continue
    }

    fn click(&mut self, click: Click) -> Flow {
        match click {
            Click::Row(index) if index < self.rows.len() => {
                self.selected = index;
                self.activate(false)
            }
            Click::Row(_) => Flow::Continue,
        }
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    /// Telmo.app's terminal keeps an old picture under the new one until the
    /// screen is cleared; so does a picture that is no longer drawn.
    fn take_clear(&mut self) -> bool {
        let now = self.drawn_images.borrow().clone();
        let stale = self.last_images.iter().any(|old| !now.contains(old));
        self.last_images = now;
        stale
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if let Some(power) = self.dialog {
            return self.dialog_key(power, key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Flow::Quit,
            KeyCode::Enter => return self.activate(false),
            KeyCode::Up | KeyCode::BackTab => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('p' | 'k') if ctrl => self.move_selection(-1),
            KeyCode::Char('n' | 'j') if ctrl => self.move_selection(1),
            KeyCode::Tab => {
                if matches!(self.selected_row(), Some(Row::Math(_))) {
                    return self.activate(true);
                }
                self.move_selection(1);
            }
            _ => {
                if self.input.handle(key) {
                    self.toast = None;
                    self.refresh(false);
                }
            }
        }
        Flow::Continue
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        if self.dialog.is_some() {
            return Flow::Continue;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                match self.hits.at(event.column, event.row) {
                    Some(click) => self.click(click),
                    None => Flow::Continue,
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                Flow::Continue
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Apps(apps) => {
                self.apps = apps;
                self.refresh(true);
            }
            Event::Running(running) => self.running = running,
            Event::Icon { id, image } => {
                self.protocols.borrow_mut().remove(&id);
                self.images.insert(id, image);
            }
            Event::Done(None) => return Flow::Quit,
            Event::Done(Some(note)) => {
                self.pending = false;
                self.toast = Some(Toast::ok(note));
            }
            Event::Failed(message) => {
                self.pending = false;
                self.error(message);
            }
        }
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.toast.is_some()
    }
}
