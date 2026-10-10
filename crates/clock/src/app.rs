//! State and key handling. No drawing here.

use crate::{
    cli::now_ms,
    config::Place,
    model::{Alarm, ClockState, TimerStatus},
    parse, store,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use jiff::tz::TimeZone;
use telmo_kit::{Flow, hits::Hits, input::TextInput, widgets::Toast};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Timers,
    Stopwatch,
    World,
    Alarms,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Timers, Tab::Stopwatch, Tab::World, Tab::Alarms];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Timers => "Timers",
            Tab::Stopwatch => "Stopwatch",
            Tab::World => "World",
            Tab::Alarms => "Alarms",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

pub enum Dialog {
    Help,
    /// `pick` is how many times Tab has offered a recent length.
    NewTimer {
        input: TextInput,
        pick: usize,
    },
    NewAlarm {
        input: TextInput,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    Tab(Tab),
    Key(KeyCode),
    Row(usize),
    Inside,
    Outside,
}

pub struct App {
    pub state: ClockState,
    pub places: Vec<Place>,
    pub tz: TimeZone,
    pub tab: Tab,
    pub timer: usize,
    pub city: usize,
    pub alarm: usize,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub hits: Hits<Click>,
    /// Tests pin the clock here.
    pub fixed_now: Option<i64>,
    tick: u64,
    /// False for `--mock`: changes stay in memory.
    persist: bool,
}

/// Sample data for `--mock`.
pub fn mock_state(now: i64) -> ClockState {
    let mut state = ClockState::default();
    state.add_timer("Tea", 300_000, now);
    state.add_timer("Pasta", 600_000, now);
    state.stopwatch.start(now - 120_000);
    state
}

impl App {
    pub fn new(state: ClockState, places: Vec<Place>, tz: TimeZone, persist: bool) -> Self {
        Self {
            state,
            places,
            tz,
            tab: Tab::Timers,
            timer: 0,
            city: 0,
            alarm: 0,
            dialog: None,
            toast: None,
            hits: Hits::default(),
            fixed_now: None,
            tick: 0,
            persist,
        }
    }

    pub fn warn(&mut self, message: String) {
        self.toast = Some(Toast::error(message));
    }

    pub fn now(&self) -> i64 {
        self.fixed_now.unwrap_or_else(now_ms)
    }

    /// Changes the state (on disk too, under the lock) and shows the result.
    fn change<R>(
        &mut self,
        change: impl FnOnce(&mut ClockState, i64, &TimeZone) -> R,
    ) -> Option<R> {
        let now = self.now();
        let tz = self.tz.clone();
        if !self.persist {
            return Some(change(&mut self.state, now, &tz));
        }
        match store::apply(now, |s| change(s, now, &tz)) {
            Ok(applied) => {
                self.state = applied.state;
                if let Some(problem) = applied.problems.first() {
                    self.toast = Some(Toast::error(format!(
                        "Saved, but no alert was set: {problem}"
                    )));
                }
                Some(applied.result)
            }
            Err(e) => {
                self.toast = Some(Toast::error(format!("Can't save: {e}")));
                None
            }
        }
    }

    fn reload(&mut self) {
        if self.persist {
            self.state = store::load();
        }
        self.clamp();
    }

    fn clamp(&mut self) {
        self.timer = self.timer.min(self.state.timers.len().saturating_sub(1));
        self.alarm = self.alarm.min(self.state.alarms.len().saturating_sub(1));
        self.city = self.city.min(self.places.len().saturating_sub(1));
    }

    fn press(&mut self, code: KeyCode) -> Flow {
        telmo_kit::App::key(self, KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn go(&mut self, offset: isize) {
        let n = Tab::ALL.len() as isize;
        let next = (self.tab.index() as isize + offset).rem_euclid(n);
        self.tab = Tab::ALL[next as usize];
    }

    fn select(&mut self, offset: isize) {
        let (value, len) = match self.tab {
            Tab::Timers => (&mut self.timer, self.state.timers.len()),
            Tab::World => (&mut self.city, self.places.len()),
            Tab::Alarms => (&mut self.alarm, self.state.alarms.len()),
            Tab::Stopwatch => return,
        };
        *value = value
            .saturating_add_signed(offset)
            .min(len.saturating_sub(1));
    }

    fn set_row(&mut self, index: usize) {
        match self.tab {
            Tab::Timers => self.timer = index,
            Tab::World => self.city = index,
            Tab::Alarms => self.alarm = index,
            Tab::Stopwatch => {}
        }
        self.clamp();
    }

    // --- actions ---

    fn toggle_timer(&mut self) {
        let Some(id) = self.state.timers.get(self.timer).map(|t| t.id.clone()) else {
            return;
        };
        self.change(|s, now, _| {
            if let Some(t) = s.timers.iter_mut().find(|t| t.id == id) {
                t.toggle(now);
            }
        });
    }

    fn restart_timer(&mut self) {
        let Some(id) = self.state.timers.get(self.timer).map(|t| t.id.clone()) else {
            return;
        };
        self.change(|s, now, _| {
            if let Some(t) = s.timers.iter_mut().find(|t| t.id == id) {
                t.restart(now);
            }
        });
    }

    fn delete_timer(&mut self) {
        let Some(timer) = self.state.timers.get(self.timer) else {
            return;
        };
        let (id, label) = (timer.id.clone(), crate::ui::timer_title(timer));
        if self
            .change(|s, _, _| s.timers.retain(|t| t.id != id))
            .is_some()
        {
            self.toast = Some(Toast::ok(format!("Deleted {label}")));
        }
        self.clamp();
    }

    fn toggle_alarm(&mut self) {
        let Some(id) = self.state.alarms.get(self.alarm).map(|a| a.id.clone()) else {
            return;
        };
        self.change(|s, now, tz| {
            if let Some(a) = s.alarms.iter_mut().find(|a| a.id == id) {
                a.toggle(now, tz);
            }
        });
    }

    fn delete_alarm(&mut self) {
        let Some(id) = self.state.alarms.get(self.alarm).map(|a| a.id.clone()) else {
            return;
        };
        if self
            .change(|s, _, _| s.alarms.retain(|a| a.id != id))
            .is_some()
        {
            self.toast = Some(Toast::ok("Deleted the alarm"));
        }
        self.clamp();
    }

    fn stopwatch_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(' ') | KeyCode::Enter => {
                self.change(|s, now, _| s.stopwatch.toggle(now));
            }
            KeyCode::Char('l') => {
                self.change(|s, now, _| s.stopwatch.lap(now));
            }
            KeyCode::Char('r') => {
                self.change(|s, _, _| s.stopwatch.reset());
            }
            _ => {}
        }
    }

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Right => self.go(1),
            KeyCode::Left => self.go(-1),
            KeyCode::Tab => self.go(1),
            KeyCode::BackTab => self.go(-1),
            KeyCode::Char(c @ '1'..='4') => self.tab = Tab::ALL[c as usize - '1' as usize],
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            KeyCode::Down | KeyCode::Char('j') => self.select(1),
            KeyCode::Up | KeyCode::Char('k') => self.select(-1),
            _ => match self.tab {
                Tab::Timers => match key.code {
                    KeyCode::Char('n') => {
                        self.dialog = Some(Dialog::NewTimer {
                            input: TextInput::default(),
                            pick: 0,
                        });
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => self.toggle_timer(),
                    KeyCode::Char('r') => self.restart_timer(),
                    KeyCode::Char('x') => self.delete_timer(),
                    _ => {}
                },
                Tab::Stopwatch => self.stopwatch_key(key),
                Tab::World => {}
                Tab::Alarms => match key.code {
                    KeyCode::Char('n') => {
                        self.dialog = Some(Dialog::NewAlarm {
                            input: TextInput::default(),
                        });
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => self.toggle_alarm(),
                    KeyCode::Char('x') => self.delete_alarm(),
                    _ => {}
                },
            },
        }
        Flow::Continue
    }

    /// Returns the dialog to keep showing.
    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Option<Dialog> {
        match dialog {
            Dialog::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::Enter => None,
                _ => Some(Dialog::Help),
            },
            Dialog::NewTimer {
                mut input,
                mut pick,
            } => match key.code {
                KeyCode::Esc => None,
                KeyCode::Enter => match parse::timer(&input.value) {
                    Ok((duration, name)) => {
                        // A failed save keeps its error toast.
                        if self
                            .change(|s, now, _| s.add_timer(&name, duration, now))
                            .is_some()
                        {
                            self.timer = self.state.timers.len().saturating_sub(1);
                            self.toast = Some(Toast::ok(format!(
                                "Started a {} timer",
                                crate::fmt::short(duration)
                            )));
                        }
                        None
                    }
                    Err(_) => Some(Dialog::NewTimer { input, pick }),
                },
                KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                    let recent = &self.state.recent;
                    if !recent.is_empty() {
                        let length = crate::fmt::short(recent[pick % recent.len()]);
                        let name = input
                            .value
                            .trim()
                            .split_once(char::is_whitespace)
                            .map_or("", |(_, n)| n.trim());
                        input.value = format!("{length} {name}").trim().to_string();
                        pick += 1;
                    }
                    Some(Dialog::NewTimer { input, pick })
                }
                _ => {
                    input.handle(key);
                    Some(Dialog::NewTimer { input, pick })
                }
            },
            Dialog::NewAlarm { mut input } => match key.code {
                KeyCode::Esc => None,
                KeyCode::Enter => match parse::alarm(&input.value) {
                    Ok(spec) => {
                        let alarm = Alarm {
                            name: spec.name,
                            hour: spec.hour,
                            minute: spec.minute,
                            days: spec.days,
                            ..Alarm::default()
                        };
                        let id = self.change(|s, now, tz| s.add_alarm(alarm, now, tz));
                        let id_saved = id.is_some();
                        if let Some(index) =
                            id.and_then(|id| self.state.alarms.iter().position(|a| a.id == id))
                        {
                            self.alarm = index;
                        }
                        if id_saved {
                            self.toast = Some(Toast::ok("Alarm added"));
                        }
                        None
                    }
                    Err(_) => Some(Dialog::NewAlarm { input }),
                },
                _ => {
                    input.handle(key);
                    Some(Dialog::NewAlarm { input })
                }
            },
        }
    }

    pub fn running_summary(&self) -> (usize, usize) {
        let now = self.now();
        let paused = self
            .state
            .timers
            .iter()
            .filter(|t| t.status(now) == TimerStatus::Paused)
            .count();
        (self.state.running_timers(now), paused)
    }
}

impl telmo_kit::App for App {
    type Event = ();

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
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                match self.hits.at(event.column, event.row) {
                    Some(Click::Tab(tab)) => {
                        self.tab = tab;
                        Flow::Continue
                    }
                    Some(Click::Key(code)) => self.press(code),
                    Some(Click::Row(index)) => {
                        self.set_row(index);
                        Flow::Continue
                    }
                    Some(Click::Outside) => self.press(KeyCode::Esc),
                    Some(Click::Inside) | None => Flow::Continue,
                }
            }
            MouseEventKind::ScrollDown if self.dialog.is_none() => self.press(KeyCode::Down),
            MouseEventKind::ScrollUp if self.dialog.is_none() => self.press(KeyCode::Up),
            _ => Flow::Continue,
        }
    }

    fn event(&mut self, _event: ()) -> Flow {
        Flow::Continue
    }

    /// A clock never rests: the digits move. Once a second the file is read
    /// again, so what `fire` or another popup changed shows up.
    fn tick(&mut self) -> Flow {
        self.tick += 1;
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        if self.tick.is_multiple_of(10) && self.dialog.is_none() {
            self.reload();
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        true
    }
}
