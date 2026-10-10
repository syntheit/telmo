//! What `clock.json` holds, and the arithmetic on it.
//!
//! Nothing here ticks. Every moment is a unix timestamp in milliseconds and
//! every question ("how long is left?") takes `now` as an argument, so a
//! closed popup changes nothing and tests use any clock they like.
//!
//! Telmo.app (macOS) reads this file too: it looks at `timers[].ends_at_ms`,
//! `timers[].fired`, `alarms[].next_ms` and `alarms[].enabled`. Keep those
//! names, and keep `crate::store` the only writer.

use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Serialize};

/// An alarm this much past its time was slept through: say so, quietly.
pub const MISSED_AFTER_MS: i64 = 120_000;
/// How early a scheduled wake-up may arrive and still count as due.
pub const DUE_SLACK_MS: i64 = 2_000;
const RECENT: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockState {
    pub timers: Vec<Timer>,
    pub stopwatch: Stopwatch,
    pub alarms: Vec<Alarm>,
    /// The last few durations started, newest first.
    pub recent: Vec<i64>,
}

// --- timers ---

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timer {
    pub id: String,
    pub name: String,
    pub duration_ms: i64,
    /// Set while running (and after it ran out until it is restarted).
    pub ends_at_ms: Option<i64>,
    /// Set while paused.
    pub paused_remaining_ms: Option<i64>,
    /// The notification went out.
    pub fired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerStatus {
    Running,
    Paused,
    Done,
}

impl Timer {
    pub fn new(id: String, name: String, duration_ms: i64, now: i64) -> Self {
        Self {
            id,
            name,
            duration_ms,
            ends_at_ms: Some(now + duration_ms),
            paused_remaining_ms: None,
            fired: false,
        }
    }

    pub fn remaining(&self, now: i64) -> i64 {
        match (self.paused_remaining_ms, self.ends_at_ms) {
            (Some(left), _) => left,
            (None, Some(end)) => (end - now).max(0),
            (None, None) => 0,
        }
    }

    pub fn status(&self, now: i64) -> TimerStatus {
        if self.paused_remaining_ms.is_some() {
            TimerStatus::Paused
        } else if self.fired || self.remaining(now) == 0 {
            TimerStatus::Done
        } else {
            TimerStatus::Running
        }
    }

    /// 0 when it just started, 1 when it ran out.
    pub fn progress(&self, now: i64) -> f32 {
        if self.duration_ms <= 0 {
            return 1.0;
        }
        let left = self.remaining(now) as f32 / self.duration_ms as f32;
        (1.0 - left).clamp(0.0, 1.0)
    }

    pub fn pause(&mut self, now: i64) {
        if self.status(now) == TimerStatus::Running {
            self.paused_remaining_ms = Some(self.remaining(now));
            self.ends_at_ms = None;
        }
    }

    pub fn resume(&mut self, now: i64) {
        if let Some(left) = self.paused_remaining_ms.take() {
            self.ends_at_ms = Some(now + left);
        }
    }

    pub fn restart(&mut self, now: i64) {
        self.paused_remaining_ms = None;
        self.ends_at_ms = Some(now + self.duration_ms);
        self.fired = false;
    }

    /// Enter: pause a running timer, resume a paused one, restart a finished one.
    pub fn toggle(&mut self, now: i64) {
        match self.status(now) {
            TimerStatus::Running => self.pause(now),
            TimerStatus::Paused => self.resume(now),
            TimerStatus::Done => self.restart(now),
        }
    }

    /// When it must fire, if it still has to.
    pub fn due_ms(&self) -> Option<i64> {
        if self.fired || self.paused_remaining_ms.is_some() {
            return None;
        }
        self.ends_at_ms
    }
}

// --- stopwatch ---

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stopwatch {
    /// Set while running.
    pub started_at_ms: Option<i64>,
    /// Time gathered before the current run began.
    pub elapsed_ms: i64,
    /// Elapsed time at each lap press, oldest first.
    pub laps: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lap {
    pub number: usize,
    pub split_ms: i64,
    pub total_ms: i64,
    /// Split minus the previous split; none for the first lap.
    pub delta_ms: Option<i64>,
}

impl Stopwatch {
    pub fn running(&self) -> bool {
        self.started_at_ms.is_some()
    }

    pub fn elapsed(&self, now: i64) -> i64 {
        self.elapsed_ms + self.started_at_ms.map_or(0, |start| (now - start).max(0))
    }

    pub fn start(&mut self, now: i64) {
        if self.started_at_ms.is_none() {
            self.started_at_ms = Some(now);
        }
    }

    pub fn stop(&mut self, now: i64) {
        self.elapsed_ms = self.elapsed(now);
        self.started_at_ms = None;
    }

    pub fn toggle(&mut self, now: i64) {
        if self.running() {
            self.stop(now);
        } else {
            self.start(now);
        }
    }

    /// Laps only count while running.
    pub fn lap(&mut self, now: i64) {
        if self.running() {
            self.laps.push(self.elapsed(now));
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Time since the last lap (or since the start).
    pub fn current_split(&self, now: i64) -> i64 {
        self.elapsed(now) - self.laps.last().copied().unwrap_or(0)
    }

    /// Oldest first.
    pub fn laps(&self) -> Vec<Lap> {
        let mut previous_total = 0;
        let mut previous_split = None;
        self.laps
            .iter()
            .enumerate()
            .map(|(i, &total_ms)| {
                let split_ms = total_ms - previous_total;
                let lap = Lap {
                    number: i + 1,
                    split_ms,
                    total_ms,
                    delta_ms: previous_split.map(|p| split_ms - p),
                };
                previous_total = total_ms;
                previous_split = Some(split_ms);
                lap
            })
            .collect()
    }
}

// --- alarms ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alarm {
    pub id: String,
    pub name: String,
    pub hour: u8,
    pub minute: u8,
    /// 1 = Monday … 7 = Sunday. Empty means once.
    pub days: Vec<u8>,
    pub enabled: bool,
    /// The next time it rings, in the zone the machine was in when this was
    /// computed. None while off.
    pub next_ms: Option<i64>,
    pub last_fired_ms: Option<i64>,
}

impl Default for Alarm {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            hour: 7,
            minute: 0,
            days: Vec::new(),
            enabled: true,
            next_ms: None,
            last_fired_ms: None,
        }
    }
}

/// The first moment after `after_ms` at `hour:minute` on one of `days`
/// (any day when empty), by the wall clock of `tz`. A time that does not
/// exist on a DST day moves forward; one that happens twice rings the first.
pub fn next_occurrence(
    hour: u8,
    minute: u8,
    days: &[u8],
    after_ms: i64,
    tz: &TimeZone,
) -> Option<i64> {
    // `Date::at` panics on an impossible time, and clock.json is hand-editable.
    if hour > 23 || minute > 59 {
        return None;
    }
    let start = Timestamp::from_millisecond(after_ms)
        .ok()?
        .to_zoned(tz.clone());
    let mut date = start.date();
    for _ in 0..8 {
        let weekday = date.weekday().to_monday_one_offset() as u8;
        if days.is_empty() || days.contains(&weekday) {
            let at = date.at(hour as i8, minute as i8, 0, 0).to_zoned(tz.clone());
            if let Ok(at) = at {
                let ms = at.timestamp().as_millisecond();
                if ms > after_ms {
                    return Some(ms);
                }
            }
        }
        date = date.tomorrow().ok()?;
    }
    None
}

impl Alarm {
    /// Works out `next_ms` from `now` (nothing while off).
    pub fn arm(&mut self, now: i64, tz: &TimeZone) {
        self.next_ms = if self.enabled {
            next_occurrence(self.hour, self.minute, &self.days, now, tz)
        } else {
            None
        };
    }

    pub fn toggle(&mut self, now: i64, tz: &TimeZone) {
        self.enabled = !self.enabled;
        self.arm(now, tz);
    }

    pub fn due_ms(&self) -> Option<i64> {
        self.next_ms.filter(|_| self.enabled)
    }
}

// --- the whole file ---

/// What announcing an item should say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub kind: Kind,
    pub name: String,
    /// Timer: its length. Alarm: "07:30".
    pub detail: String,
    /// An alarm that was slept through.
    pub missed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Timer,
    Alarm,
}

impl ClockState {
    /// An id no item has, from the creation time.
    pub fn fresh_id(&self, now: i64) -> String {
        let mut n = now.rem_euclid(0xffff_ffff) as u32;
        loop {
            let id = format!("{n:08x}");
            let taken =
                self.timers.iter().any(|t| t.id == id) || self.alarms.iter().any(|a| a.id == id);
            if !taken {
                return id;
            }
            n = n.wrapping_add(1);
        }
    }

    pub fn add_timer(&mut self, name: &str, duration_ms: i64, now: i64) -> String {
        let id = self.fresh_id(now);
        self.timers.push(Timer::new(
            id.clone(),
            name.trim().to_string(),
            duration_ms,
            now,
        ));
        self.recent.retain(|&d| d != duration_ms);
        self.recent.insert(0, duration_ms);
        self.recent.truncate(RECENT);
        id
    }

    pub fn add_alarm(&mut self, mut alarm: Alarm, now: i64, tz: &TimeZone) -> String {
        alarm.id = self.fresh_id(now);
        alarm.arm(now, tz);
        let id = alarm.id.clone();
        self.alarms.push(alarm);
        self.alarms.sort_by_key(|a| (a.hour, a.minute));
        id
    }

    /// Every id that has to be woken at some moment, and when.
    pub fn due(&self) -> Vec<(String, i64)> {
        let timers = self
            .timers
            .iter()
            .filter_map(|t| Some((t.id.clone(), t.due_ms()?)));
        let alarms = self
            .alarms
            .iter()
            .filter_map(|a| Some((a.id.clone(), a.due_ms()?)));
        timers.chain(alarms).collect()
    }

    pub fn running_timers(&self, now: i64) -> usize {
        self.timers
            .iter()
            .filter(|t| t.status(now) == TimerStatus::Running)
            .count()
    }

    /// Marks an item announced, if it is really due: a timer stays as "done",
    /// an alarm moves on to its next day (or switches off if it was one-off).
    /// Returns what to announce, or None when there was nothing to do (already
    /// fired, paused or restarted since, or not due yet).
    pub fn mark_fired(&mut self, id: &str, now: i64, tz: &TimeZone) -> Option<Fired> {
        if let Some(timer) = self.timers.iter_mut().find(|t| t.id == id) {
            if timer.due_ms()? > now + DUE_SLACK_MS {
                return None;
            }
            timer.fired = true;
            return Some(Fired {
                kind: Kind::Timer,
                name: timer.name.clone(),
                detail: crate::fmt::short(timer.duration_ms),
                missed: false,
            });
        }
        let alarm = self.alarms.iter_mut().find(|a| a.id == id)?;
        let due = alarm.due_ms()?;
        if due > now + DUE_SLACK_MS {
            return None;
        }
        alarm.last_fired_ms = Some(now);
        if alarm.days.is_empty() {
            alarm.enabled = false;
            alarm.next_ms = None;
        } else {
            alarm.next_ms =
                next_occurrence(alarm.hour, alarm.minute, &alarm.days, now.max(due), tz);
        }
        Some(Fired {
            kind: Kind::Alarm,
            name: alarm.name.clone(),
            detail: format!("{:02}:{:02}", alarm.hour, alarm.minute),
            missed: now - due > MISSED_AFTER_MS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000_000;

    fn zone(name: &str) -> TimeZone {
        TimeZone::get(name).expect("zone")
    }

    fn ms(zoned: &str) -> i64 {
        zoned
            .parse::<jiff::Zoned>()
            .expect("zoned")
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn timer_counts_down_with_the_clock() {
        let t = Timer::new("a".into(), "tea".into(), 300_000, T0);
        assert_eq!(t.remaining(T0), 300_000);
        assert_eq!(t.remaining(T0 + 120_000), 180_000);
        assert_eq!(t.status(T0 + 120_000), TimerStatus::Running);
        assert_eq!(t.remaining(T0 + 400_000), 0);
        assert_eq!(t.status(T0 + 400_000), TimerStatus::Done);
        assert!((t.progress(T0 + 150_000) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn timer_pauses_and_resumes_without_losing_time() {
        let mut t = Timer::new("a".into(), String::new(), 300_000, T0);
        t.pause(T0 + 100_000);
        assert_eq!(t.status(T0 + 100_000), TimerStatus::Paused);
        // An hour later it still has 200 s.
        assert_eq!(t.remaining(T0 + 3_700_000), 200_000);
        assert_eq!(t.due_ms(), None);
        t.resume(T0 + 3_700_000);
        assert_eq!(t.remaining(T0 + 3_700_000), 200_000);
        assert_eq!(t.due_ms(), Some(T0 + 3_900_000));
        // Pausing twice changes nothing.
        t.pause(T0 + 3_800_000);
        t.pause(T0 + 3_850_000);
        assert_eq!(t.remaining(T0 + 9_000_000), 100_000);
    }

    #[test]
    fn timer_toggle_and_restart() {
        let mut t = Timer::new("a".into(), String::new(), 60_000, T0);
        t.toggle(T0 + 10_000);
        assert_eq!(t.status(T0 + 10_000), TimerStatus::Paused);
        t.toggle(T0 + 20_000);
        assert_eq!(t.status(T0 + 20_000), TimerStatus::Running);
        t.fired = true;
        assert_eq!(t.status(T0 + 20_000), TimerStatus::Done);
        t.toggle(T0 + 30_000);
        assert!(!t.fired);
        assert_eq!(t.remaining(T0 + 30_000), 60_000);
    }

    #[test]
    fn stopwatch_accumulates_across_stops() {
        let mut s = Stopwatch::default();
        s.start(T0);
        assert_eq!(s.elapsed(T0 + 5_000), 5_000);
        s.stop(T0 + 5_000);
        assert_eq!(s.elapsed(T0 + 99_000), 5_000);
        s.start(T0 + 100_000);
        assert_eq!(s.elapsed(T0 + 103_000), 8_000);
        s.reset();
        assert_eq!(s.elapsed(T0 + 103_000), 0);
        assert!(!s.running());
    }

    #[test]
    fn laps_have_splits_and_deltas() {
        let mut s = Stopwatch::default();
        s.lap(T0); // not running: ignored
        s.start(T0);
        s.lap(T0 + 10_000);
        s.lap(T0 + 25_000);
        s.lap(T0 + 33_000);
        let laps = s.laps();
        assert_eq!(laps.len(), 3);
        assert_eq!(laps[0].delta_ms, None);
        assert_eq!((laps[1].split_ms, laps[1].delta_ms), (15_000, Some(5_000)));
        assert_eq!((laps[2].split_ms, laps[2].delta_ms), (8_000, Some(-7_000)));
        assert_eq!(laps[2].total_ms, 33_000);
        assert_eq!(s.current_split(T0 + 40_000), 7_000);
    }

    #[test]
    fn alarm_picks_the_next_weekday() {
        let ny = zone("America/New_York");
        // Saturday 2026-10-10 14:07; a weekday alarm rings on Monday.
        let now = ms("2026-10-10T14:07[America/New_York]");
        let next = next_occurrence(7, 30, &[1, 2, 3, 4, 5], now, &ny);
        assert_eq!(next, Some(ms("2026-10-12T07:30[America/New_York]")));
        // Later today when it has not happened yet; tomorrow when it has.
        assert_eq!(
            next_occurrence(18, 0, &[], now, &ny),
            Some(ms("2026-10-10T18:00[America/New_York]"))
        );
        assert_eq!(
            next_occurrence(9, 0, &[], now, &ny),
            Some(ms("2026-10-11T09:00[America/New_York]"))
        );
        // Exactly now does not count.
        assert_eq!(
            next_occurrence(14, 7, &[], now, &ny),
            Some(ms("2026-10-11T14:07[America/New_York]"))
        );
        // Only Sundays.
        assert_eq!(
            next_occurrence(8, 0, &[7], now, &ny),
            Some(ms("2026-10-11T08:00[America/New_York]"))
        );
        // A hand-edited file with an impossible time is never due, not a panic.
        assert_eq!(next_occurrence(24, 0, &[], now, &ny), None);
        assert_eq!(next_occurrence(255, 255, &[], now, &ny), None);
        assert_eq!(next_occurrence(7, 60, &[], now, &ny), None);
    }

    #[test]
    fn alarm_keeps_the_wall_clock_across_dst() {
        let ny = zone("America/New_York");
        // Clocks go forward on 2026-03-08: 07:30 is 12:30 UTC before, 11:30 UTC after.
        let before = ms("2026-03-07T08:00[America/New_York]");
        let next = next_occurrence(7, 30, &[], before, &ny).expect("next");
        assert_eq!(next, ms("2026-03-08T07:30[America/New_York]"));
        assert_eq!(next - before, 23 * 3_600_000 + 30 * 60_000 - 3_600_000);
        // 02:30 does not exist that day: it rings at 03:30.
        let gap = next_occurrence(2, 30, &[], before, &ny).expect("gap");
        assert_eq!(gap, ms("2026-03-08T03:30[America/New_York]"));
        // Clocks go back on 2026-11-01: 01:30 happens twice, the first counts.
        let fall = ms("2026-10-31T23:00[America/New_York]");
        let first = next_occurrence(1, 30, &[], fall, &ny).expect("fall");
        assert_eq!(first, ms("2026-11-01T01:30-04:00[America/New_York]"));
    }

    #[test]
    fn firing_moves_alarms_on_and_marks_timers() {
        let tz = zone("America/Argentina/Buenos_Aires");
        let now = ms("2026-10-10T14:07[America/Argentina/Buenos_Aires]");
        let mut state = ClockState::default();
        let timer = state.add_timer("tea", 300_000, now);
        let daily = state.add_alarm(
            Alarm {
                name: "pills".into(),
                hour: 21,
                minute: 0,
                days: vec![1, 2, 3, 4, 5, 6, 7],
                ..Alarm::default()
            },
            now,
            &tz,
        );
        let once = state.add_alarm(
            Alarm {
                name: "flight".into(),
                hour: 15,
                minute: 0,
                ..Alarm::default()
            },
            now,
            &tz,
        );
        assert_eq!(state.due().len(), 3);

        // Not due yet: nothing happens.
        assert_eq!(state.mark_fired(&timer, now, &tz), None);

        let fired = state.mark_fired(&timer, now + 300_000, &tz).expect("fired");
        assert_eq!(
            (fired.kind, fired.name.as_str(), fired.detail.as_str()),
            (Kind::Timer, "tea", "5m")
        );
        assert_eq!(
            state.mark_fired(&timer, now + 301_000, &tz),
            None,
            "only once"
        );

        let due = ms("2026-10-10T21:00[America/Argentina/Buenos_Aires]");
        let fired = state.mark_fired(&daily, due, &tz).expect("alarm");
        assert!(!fired.missed);
        let alarm = state.alarms.iter().find(|a| a.id == daily).expect("alarm");
        assert_eq!(
            alarm.next_ms,
            Some(ms("2026-10-11T21:00[America/Argentina/Buenos_Aires]"))
        );

        // One-off alarms switch themselves off, and a slept-through one says so.
        let late = ms("2026-10-10T15:30[America/Argentina/Buenos_Aires]");
        let fired = state.mark_fired(&once, late, &tz).expect("once");
        assert!(fired.missed);
        let alarm = state.alarms.iter().find(|a| a.id == once).expect("alarm");
        assert!(!alarm.enabled && alarm.next_ms.is_none());
    }

    #[test]
    fn a_paused_or_restarted_timer_is_not_fired() {
        let tz = TimeZone::UTC;
        let mut state = ClockState::default();
        let id = state.add_timer("", 60_000, T0);
        state.timers[0].pause(T0 + 10_000);
        assert_eq!(state.mark_fired(&id, T0 + 70_000, &tz), None);
        state.timers[0].resume(T0 + 80_000);
        assert_eq!(state.mark_fired(&id, T0 + 100_000, &tz), None);
    }

    #[test]
    fn recent_durations_are_distinct_newest_first_and_three_long() {
        let mut state = ClockState::default();
        for d in [60_000, 300_000, 60_000, 90_000, 600_000] {
            state.add_timer("", d, T0);
        }
        assert_eq!(state.recent, vec![600_000, 90_000, 60_000]);
    }

    #[test]
    fn ids_are_unique_within_one_millisecond() {
        let mut state = ClockState::default();
        let a = state.add_timer("", 1000, T0);
        let b = state.add_timer("", 1000, T0);
        assert_ne!(a, b);
    }

    #[test]
    fn state_round_trips_and_accepts_old_files() {
        let tz = TimeZone::UTC;
        let mut state = ClockState::default();
        state.add_timer("tea", 300_000, T0);
        state.stopwatch.start(T0);
        state.stopwatch.lap(T0 + 5);
        state.add_alarm(
            Alarm {
                name: "a".into(),
                days: vec![1, 3],
                ..Alarm::default()
            },
            T0,
            &tz,
        );
        let json = serde_json::to_string(&state).expect("json");
        assert_eq!(
            serde_json::from_str::<ClockState>(&json).expect("back"),
            state
        );
        let empty: ClockState = serde_json::from_str("{}").expect("empty");
        assert_eq!(empty, ClockState::default());
    }
}
