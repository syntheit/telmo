//! State and key handling. No drawing here.

use crate::actions::{Cmd, Event};
use crate::canvas::Canvas;
use crate::effects::{self, Effect, Frame, Logo, Transition};
use crate::prefs::{self, Prefs};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use std::{
    cell::Cell,
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
    pending: bool,
    persist: bool,
    save_warned: bool,
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

impl App {
    pub fn new(cmds: UnboundedSender<Cmd>, resolved: prefs::Resolved, host: String) -> Self {
        let (width, height) = crossterm::terminal::size().unwrap_or((90, 22));
        Self::build(cmds, resolved, host, (width, height), true, false)
    }

    #[cfg(test)]
    pub fn for_test(cmds: UnboundedSender<Cmd>, size: (u16, u16)) -> Self {
        let resolved = prefs::resolve(None, None);
        let mut app = Self::build(cmds, resolved, "swift".into(), size, false, true);
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
    ) -> Self {
        let logo = Logo::place(resolved.prefs.logo, size.0, size.1.saturating_sub(1));
        let mut app = Self {
            effect: Box::new(Plain),
            transition: Transition::new(&logo),
            canvas: Canvas::new(size.0, size.1.saturating_sub(1)),
            prefs: resolved.prefs,
            cycle: resolved.cycle,
            host,
            dialog: None,
            toast: None,
            now: 0.0,
            switched_at: 0.0,
            logo,
            hits: Hits::default(),
            area: Cell::new(size),
            effect_t: 0.0,
            last: Instant::now(),
            cmds,
            pending: false,
            persist,
            save_warned: false,
            plain,
        };
        app.rebuild();
        app.advance(0.0);
        app
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
            busy: false,
            finished: false,
        };
        self.effect.frame(&mut frame);
        self.transition
            .apply(&mut self.canvas, &self.logo, self.now - self.switched_at);
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
        self.advance(dt);
        Flow::Continue
    }

    fn animating(&self) -> bool {
        true
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
