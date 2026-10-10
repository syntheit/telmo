//! State and key handling. No drawing here.

use crate::actions::{Cmd, Event};
use crate::apps::{
    AppCmd,
    view::{Action, View},
};
use crate::canvas::Canvas;
use crate::effects::{self, Effect, Frame, Logo, Transition};
use crate::prefs::{self, Prefs};
use crate::rebuild::{self, State, Status};
use crate::tracker::{Change, Tracker};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use std::{
    cell::Cell,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use telmo_kit::{Flow, hits::Hits, widgets::Toast};
use tokio::sync::mpsc::UnboundedSender;

/// How long the effect name and dots stay after a switch.
pub const SWITCH_TOAST: f32 = 1.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Help,
    Confirm(Cmd),
}

/// What a click on a drawn area does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    Logo,
    /// A row of the Force Quit list.
    AppRow(i32),
    Prev,
    Next,
    /// The confirm dialog's `↵` hint.
    Confirm,
    /// Inside a dialog: swallows the click.
    Inside,
    /// Outside the open dialog: closes it.
    Outside,
}

pub struct App {
    pub prefs: Prefs,
    pub cycle: Vec<String>,
    pub host: String,
    pub canvas: Canvas,
    pub dialog: Option<Dialog>,
    /// The Force Quit view, drawn over the logo while open.
    pub view: Option<View>,
    pub toast: Option<Toast>,
    /// Seconds since start, advanced by `advance`.
    pub now: f32,
    pub switched_at: f32,
    pub logo: Logo,
    pub hits: Hits<Click>,
    /// The popup size seen by the last draw; `advance` resizes to match.
    pub area: Cell<(u16, u16)>,
    effect: Box<dyn Effect>,
    transition: Transition,
    effect_t: f32,
    last: Instant,
    cmds: UnboundedSender<Cmd>,
    apps: Option<Sender<AppCmd>>,
    pending: bool,
    persist: bool,
    save_warned: bool,
    /// The command `u` runs, from `system.json`.
    rebuild_command: Option<Vec<String>>,
    pub rebuild: Tracker,
    /// Seconds until the state file is read again.
    poll_in: f32,
    /// Set for the one frame after a rebuild finished.
    finished: bool,
    /// `--mock`: `u` plays a fake rebuild that started at this app time.
    mock: bool,
    mock_start: Option<f32>,
    /// The sudo command `u` wants the terminal for.
    command: Option<std::process::Command>,
    /// Tests draw only the logo, so snapshots don't depend on effect randomness.
    plain: bool,
}

/// Draws just the logo.
struct Plain;

impl Effect for Plain {
    fn frame(&mut self, f: &mut Frame) {
        f.logo.draw(f.canvas);
    }
}

/// `--mock` rebuild: waiting for Touch ID, then 120 builds, ending at 12 s.
const MOCK_SECONDS: f32 = 12.0;
const MOCK_START: u64 = 1_000;
const MOCK_WAITING: &str = "waiting for Touch ID…";

fn mock_status(elapsed: f32) -> Status {
    let mut status = Status::running(1, MOCK_START, MOCK_WAITING);
    if elapsed >= MOCK_SECONDS {
        status.state = State::Ok;
        status.finished = Some(MOCK_START + MOCK_SECONDS as u64);
        status.generation = Some(279);
        status.built = 120;
        status.to_build = 120;
    } else if elapsed >= 2.0 {
        status.to_build = 120;
        status.built = ((elapsed - 2.0) * 12.0) as u32;
        status.last_line = "building '/nix/store/abc-example.drv'...".into();
    }
    status
}

impl App {
    pub fn new(cmds: UnboundedSender<Cmd>, resolved: prefs::Resolved, host: String) -> Self {
        let (width, height) = crossterm::terminal::size().unwrap_or((90, 22));
        Self::build(cmds, resolved, host, (width, height), true, false, false)
    }

    pub fn with_apps(mut self, apps: Sender<AppCmd>) -> Self {
        self.apps = Some(apps);
        self
    }

    #[cfg(test)]
    pub fn for_test(cmds: UnboundedSender<Cmd>, size: (u16, u16)) -> Self {
        let resolved = prefs::resolve(None, None);
        let mut app = Self::build(cmds, resolved, "swift".into(), size, false, true, true);
        app.switched_at = -10.0;
        app.advance(0.0);
        app
    }

    fn build(
        cmds: UnboundedSender<Cmd>,
        resolved: prefs::Resolved,
        host: String,
        size: (u16, u16),
        persist: bool,
        plain: bool,
        mock: bool,
    ) -> Self {
        let logo = Logo::place(resolved.prefs.logo, size.0, size.1.saturating_sub(1));
        let mut app = Self {
            rebuild_command: resolved.rebuild,
            rebuild: Tracker::new(host.clone()),
            poll_in: 0.0,
            finished: false,
            mock,
            mock_start: None,
            command: None,
            effect: Box::new(Plain),
            transition: Transition::new(&logo),
            canvas: Canvas::new(size.0, size.1.saturating_sub(1)),
            prefs: resolved.prefs,
            cycle: resolved.cycle,
            host,
            dialog: None,
            view: None,
            toast: None,
            now: 0.0,
            switched_at: 0.0,
            logo,
            hits: Hits::default(),
            area: Cell::new(size),
            effect_t: 0.0,
            last: Instant::now(),
            cmds,
            apps: None,
            pending: false,
            persist,
            save_warned: false,
            plain,
        };
        app.rebuild();
        app.advance(0.0);
        app
    }

    /// `--mock`: `u` plays a fake rebuild instead of running the real one.
    pub fn use_mock_rebuild(&mut self) {
        self.mock = true;
    }

    #[cfg(test)]
    pub fn use_real_rebuild(&mut self, command: Vec<String>) {
        self.mock = false;
        self.rebuild_command = Some(command);
    }

    #[cfg(test)]
    pub fn finished_pending(&self) -> bool {
        self.finished
    }

    /// Places the logo again and starts a fresh effect and transition.
    fn rebuild(&mut self) {
        let (width, height) = self.area.get();
        let height = height.saturating_sub(1);
        self.canvas = Canvas::new(width, height);
        self.logo = Logo::place(self.prefs.logo, width, height);
        self.effect = if self.plain {
            Box::new(Plain)
        } else {
            effects::make(&self.prefs.effect, &self.logo)
        };
        self.transition = Transition::new(&self.logo);
        self.effect_t = 0.0;
    }

    /// Draws the next canvas frame, `dt` seconds after the previous one.
    pub fn advance(&mut self, dt: f32) {
        if self.area.get() != (self.canvas.width, self.canvas.height + 1) {
            self.rebuild();
        }
        self.now += dt;
        self.effect_t += dt;
        self.poll_rebuild(dt);
        self.canvas.clear();
        // A terminal with no room for the effect (e.g. one row tall) draws nothing.
        if self.canvas.width == 0 || self.canvas.height == 0 {
            return;
        }
        let mut frame = Frame {
            canvas: &mut self.canvas,
            logo: &mut self.logo,
            t: self.effect_t,
            dt,
            busy: self.rebuild.running(),
            finished: std::mem::take(&mut self.finished),
        };
        self.effect.frame(&mut frame);
        self.transition
            .apply(&mut self.canvas, &self.logo, self.now - self.switched_at);
    }

    /// Reads the rebuild's progress about once a second.
    fn poll_rebuild(&mut self, dt: f32) {
        self.poll_in -= dt;
        if self.poll_in > 0.0 {
            return;
        }
        self.poll_in = 1.0;
        let unix = telmo_kit::time::unix_now();
        let change = if self.mock {
            let Some(start) = self.mock_start else { return };
            let elapsed = self.now - start;
            if elapsed >= MOCK_SECONDS {
                self.mock_start = None;
            }
            self.rebuild
                .apply(mock_status(elapsed), MOCK_START + elapsed as u64, self.now)
        } else {
            let Some(status) = rebuild::read() else {
                return;
            };
            self.rebuild.apply(status, unix, self.now)
        };
        match change {
            Some(Change::Finished) => self.finished = true,
            Some(Change::Failed(message)) => self.toast = Some(Toast::error(message)),
            None => {}
        }
    }

    /// `u`: asks for the terminal to authenticate; the rebuild then keeps going
    /// after the popup closes.
    fn start_rebuild(&mut self) {
        let result = if self.rebuild.running() {
            Err(rebuild::ALREADY_RUNNING.to_string())
        } else if self.mock {
            self.mock_start = Some(self.now);
            self.rebuild.launch(self.now, MOCK_START, MOCK_WAITING);
            Ok(())
        } else if self.rebuild_command.is_none() {
            Err(rebuild::NOT_CONFIGURED.to_string())
        } else {
            let configured = self.rebuild_command.as_deref().unwrap_or_default();
            rebuild::sudo_command(configured, &self.host)
                .map(|command| self.command = Some(command))
        };
        if let Err(message) = result {
            self.toast = Some(Toast::error(message));
        }
    }

    /// `L`: opens the rebuild log.
    fn open_rebuild_log(&mut self) {
        if self.mock {
            self.toast = Some(Toast::ok("Would open the rebuild log."));
            return;
        }
        let opened = rebuild::log_path()
            .filter(|path| path.exists())
            .ok_or_else(|| "There's no rebuild log yet. Press u to rebuild.".to_string())
            .and_then(|path| telmo_kit::os::open(&path.to_string_lossy()));
        if let Err(message) = opened {
            self.toast = Some(Toast::error(message));
        }
    }

    fn restart_clock(&mut self) {
        self.switched_at = self.now;
        self.rebuild();
    }

    fn save_prefs(&mut self) {
        if !self.persist {
            return;
        }
        if let Err(e) = prefs::save(&self.prefs)
            && !self.save_warned
        {
            self.save_warned = true;
            self.toast = Some(Toast::error(format!(
                "Could not save your choice ({e}). It will reset next time."
            )));
        }
    }

    pub fn switch(&mut self, step: isize) {
        let position = self
            .cycle
            .iter()
            .position(|n| *n == self.prefs.effect)
            .unwrap_or(0);
        let next = (position as isize + step).rem_euclid(self.cycle.len() as isize) as usize;
        self.prefs.effect = self.cycle[next].clone();
        self.save_prefs();
        self.restart_clock();
    }

    fn toggle_logo(&mut self) {
        self.prefs.logo = self.prefs.logo.other();
        self.save_prefs();
        self.restart_clock();
    }

    fn send(&mut self, cmd: Cmd) {
        if self.pending {
            return;
        }
        if self.cmds.send(cmd).is_err() {
            self.toast = Some(Toast::error(
                "Lost the system backend. Close telmo-system and open it again.",
            ));
            return;
        }
        self.pending = true;
    }

    fn send_apps(&mut self, cmd: AppCmd) {
        let lost = self
            .apps
            .as_ref()
            .is_some_and(|apps| apps.send(cmd).is_err());
        if lost {
            self.toast = Some(Toast::error(
                "Lost the app list. Close telmo-system and open it again.",
            ));
        }
    }

    fn open_apps(&mut self) {
        self.view = Some(View::default());
        self.send_apps(AppCmd::Watch(true));
    }

    fn close_apps(&mut self) {
        self.view = None;
        self.send_apps(AppCmd::Watch(false));
    }

    fn view_key(&mut self, key: KeyEvent) {
        let Some(view) = self.view.as_mut() else {
            return;
        };
        match view.key(key) {
            Action::None => {}
            Action::Close => self.close_apps(),
            Action::Quit { pid, name, force } => self.send_apps(AppCmd::Quit { pid, name, force }),
        }
    }

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Left => self.switch(-1),
            KeyCode::Right => self.switch(1),
            KeyCode::Char(' ') => self.toggle_logo(),
            KeyCode::Char('l') => self.send(Cmd::Lock),
            KeyCode::Char('s') => self.send(Cmd::Sleep),
            KeyCode::Char('r') => self.dialog = Some(Dialog::Confirm(Cmd::Restart)),
            KeyCode::Char('p') => self.dialog = Some(Dialog::Confirm(Cmd::ShutDown)),
            KeyCode::Char('u') => self.start_rebuild(),
            KeyCode::Char('L') => self.open_rebuild_log(),
            KeyCode::Char('k') => self.open_apps(),
            KeyCode::Char('o') => self.dialog = Some(Dialog::Confirm(Cmd::LogOut)),
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            _ => {}
        }
        Flow::Continue
    }

    /// Handles a key for the open dialog; returns the dialog to keep showing.
    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Option<Dialog> {
        match (dialog, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q')) => None,
            (Dialog::Help, KeyCode::Char('?') | KeyCode::Enter) => None,
            (Dialog::Confirm(cmd), KeyCode::Enter) => {
                self.send(cmd);
                None
            }
            (dialog, _) => Some(dialog),
        }
    }

    fn press(&mut self, code: KeyCode) -> Flow {
        telmo_kit::App::key(
            self,
            KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        )
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if self.view.is_some() {
            self.view_key(key);
            return Flow::Continue;
        }
        match self.dialog.take() {
            Some(dialog) => {
                self.dialog = self.dialog_key(dialog, key);
                Flow::Continue
            }
            None => self.main_key(key),
        }
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Flow::Continue;
        }
        match self.hits.at(event.column, event.row) {
            Some(Click::AppRow(pid)) => {
                if let Some(view) = self.view.as_mut().filter(|v| v.confirm.is_none()) {
                    view.select(pid);
                }
                Flow::Continue
            }
            Some(Click::Logo) => self.press(KeyCode::Right),
            Some(Click::Prev) => self.press(KeyCode::Left),
            Some(Click::Next) => self.press(KeyCode::Right),
            Some(Click::Confirm) => self.press(KeyCode::Enter),
            Some(Click::Outside) => self.press(KeyCode::Esc),
            Some(Click::Inside) | None => Flow::Continue,
        }
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Apps(result) => {
                if let Some(view) = self.view.as_mut() {
                    view.set_rows(result);
                }
            }
            Event::AppNote { pid, message, ok } => {
                if let Some(view) = self.view.as_mut() {
                    view.stop_quitting(pid);
                }
                self.toast = Some(if ok {
                    Toast::ok(message)
                } else {
                    Toast::error(message)
                });
            }
            Event::Done(_) => return Flow::Quit,
            Event::Failed(message) => {
                self.pending = false;
                self.toast = Some(Toast::error(message));
            }
            Event::Note(message) => {
                self.pending = false;
                self.toast = Some(Toast::ok(message));
            }
        }
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        if let Some(view) = self.view.as_mut()
            && let Some(message) = view.tick(Instant::now()).pop()
        {
            self.toast = Some(Toast::error(message));
        }
        self.advance(dt);
        Flow::Continue
    }

    fn animating(&self) -> bool {
        true
    }

    fn take_command(&mut self) -> Option<std::process::Command> {
        self.command.take()
    }

    fn command_finished(&mut self, result: std::io::Result<std::process::ExitStatus>) -> Flow {
        match result {
            Ok(status) if status.success() => {
                self.rebuild
                    .launch(self.now, telmo_kit::time::unix_now(), rebuild::STARTING);
            }
            Ok(_) => self.toast = Some(Toast::error(rebuild::AUTH_FAILED)),
            Err(e) => {
                self.toast = Some(Toast::error(format!(
                    "Couldn't run sudo ({e}). Press u to try again."
                )));
            }
        }
        Flow::Continue
    }

    fn frame_interval(&self) -> Duration {
        Duration::from_millis(33)
    }
}

/// The machine's name without a trailing `.local`.
pub fn host_name() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates
    // unless the name was truncated, and we cut at the first NUL or the end.
    let ok = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0;
    if !ok {
        return "this computer".into();
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
    name.strip_suffix(".local").unwrap_or(&name).to_string()
}
