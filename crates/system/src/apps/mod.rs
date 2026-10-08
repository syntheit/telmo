//! The Force Quit view's data: a list of user-facing apps with memory and CPU,
//! refreshed once a second by a thread while the view is open.

pub mod draw;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod mock;
pub mod view;

#[cfg(not(target_os = "macos"))]
use linux as system;
#[cfg(target_os = "macos")]
use macos as system;

use crate::actions::Event;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

const REFRESH: Duration = Duration::from_secs(1);

/// UI -> backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCmd {
    /// Start or stop refreshing the list.
    Watch(bool),
    Quit {
        pid: i32,
        name: String,
        force: bool,
    },
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppRow {
    pub pid: i32,
    pub name: String,
    /// Bytes. On macOS the same number Activity Monitor calls Memory.
    pub memory: u64,
    /// Percent of one core; `None` until a second sample exists.
    pub cpu: Option<f32>,
    pub unresponsive: bool,
    /// Open windows, when the system tells us (Linux only).
    pub windows: u32,
}

/// What a platform reports about one app at one moment.
#[derive(Debug, Clone)]
struct Raw {
    pid: i32,
    name: String,
    memory: u64,
    /// Total CPU time used so far, in nanoseconds.
    cpu_ns: u64,
    unresponsive: bool,
    windows: u32,
}

/// Turns raw samples into rows by diffing CPU time against the last sample.
#[derive(Default)]
struct Sampler {
    last: HashMap<i32, u64>,
    at: Option<Instant>,
}

impl Sampler {
    fn rows(&mut self, raws: Vec<Raw>, now: Instant) -> Vec<AppRow> {
        let elapsed = self.at.map(|at| now.saturating_duration_since(at));
        let mut rows: Vec<AppRow> = raws
            .iter()
            .map(|raw| AppRow {
                pid: raw.pid,
                name: raw.name.clone(),
                memory: raw.memory,
                cpu: elapsed
                    .zip(self.last.get(&raw.pid))
                    .map(|(elapsed, before)| cpu_percent(*before, raw.cpu_ns, elapsed)),
                unresponsive: raw.unresponsive,
                windows: raw.windows,
            })
            .collect();
        self.last = raws.iter().map(|raw| (raw.pid, raw.cpu_ns)).collect();
        self.at = Some(now);
        sort_rows(&mut rows);
        rows
    }
}

/// CPU use between two samples, in percent of one core.
fn cpu_percent(before_ns: u64, now_ns: u64, elapsed: Duration) -> f32 {
    if elapsed.is_zero() {
        return 0.0;
    }
    now_ns.saturating_sub(before_ns) as f32 / elapsed.as_nanos() as f32 * 100.0
}

/// Not responding first, then biggest memory, then by name.
pub fn sort_rows(rows: &mut [AppRow]) {
    rows.sort_by(|a, b| {
        b.unresponsive
            .cmp(&a.unresponsive)
            .then(b.memory.cmp(&a.memory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// The rows whose name contains `query`, ignoring case.
pub fn filter_rows<'a>(rows: &'a [AppRow], query: &str) -> Vec<&'a AppRow> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| row.name.to_lowercase().contains(&query))
        .collect()
}

/// `380 MB`, `1.9 GB`.
pub fn format_memory(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes >= KB * KB * KB {
        format!("{:.1} GB", bytes / (KB * KB * KB))
    } else if bytes >= KB * KB {
        format!("{:.0} MB", bytes / (KB * KB))
    } else {
        format!("{:.0} KB", bytes / KB)
    }
}

type Refresh = Box<dyn FnMut() -> Result<Vec<AppRow>, String> + Send>;

fn source(mock: bool) -> Refresh {
    if mock {
        return Box::new(|| Ok(mock::rows()));
    }
    let mut sampler = Sampler::default();
    Box::new(move || system::list().map(|raws| sampler.rows(raws, Instant::now())))
}

/// Starts the backend thread. It ends when the returned sender is dropped.
pub fn spawn(mock: bool, events: UnboundedSender<Event>) -> Sender<AppCmd> {
    let (tx, rx) = channel();
    std::thread::spawn(move || watch(mock, rx, events));
    tx
}

fn watch(mock: bool, cmds: Receiver<AppCmd>, events: UnboundedSender<Event>) {
    let mut refresh = source(mock);
    let mut watching = false;
    loop {
        let next = if watching {
            cmds.recv_timeout(REFRESH)
        } else {
            cmds.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let event = match next {
            Ok(AppCmd::Watch(on)) => {
                watching = on;
                if !on {
                    continue;
                }
                // A fresh sampler: CPU shows "—" until the next second.
                refresh = source(mock);
                Some(Event::Apps(refresh()))
            }
            Ok(AppCmd::Quit { pid, name, force }) => quit(mock, pid, &name, force),
            Err(RecvTimeoutError::Timeout) => Some(Event::Apps(refresh())),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if let Some(event) = event
            && events.send(event).is_err()
        {
            return;
        }
    }
}

/// Success sends nothing: the app vanishing from the list is the answer.
fn quit(mock: bool, pid: i32, name: &str, force: bool) -> Option<Event> {
    if mock {
        let how = if force { "force quit" } else { "quit" };
        return Some(Event::AppNote {
            pid,
            message: format!("Would {how} {name}."),
            ok: true,
        });
    }
    system::quit(pid, name, force)
        .err()
        .map(|message| Event::AppNote {
            pid,
            message,
            ok: false,
        })
}

/// For the hidden `apps` subcommand: two real samples a second apart.
pub fn sample_once() -> Result<Vec<AppRow>, String> {
    let mut refresh = source(false);
    refresh()?;
    std::thread::sleep(REFRESH);
    refresh()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, memory: u64, unresponsive: bool) -> AppRow {
        AppRow {
            pid: 1,
            name: name.into(),
            memory,
            cpu: None,
            unresponsive,
            windows: 0,
        }
    }

    #[test]
    fn sorts_unresponsive_first_then_memory() {
        let mut rows = vec![
            row("Big", 900, false),
            row("Stuck", 10, true),
            row("Small", 100, false),
            row("alpha", 100, false),
        ];
        sort_rows(&mut rows);
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Stuck", "Big", "alpha", "Small"]);
    }

    #[test]
    fn cpu_is_time_used_over_time_passed() {
        let half_a_second = 500_000_000;
        let percent = cpu_percent(1_000, 1_000 + half_a_second, Duration::from_secs(1));
        assert!((percent - 50.0).abs() < 0.01);
        // Two busy cores can pass 100%.
        assert!(cpu_percent(0, 2_000_000_000, Duration::from_secs(1)) > 150.0);
        assert_eq!(cpu_percent(5, 5, Duration::ZERO), 0.0);
        // A counter that went backwards (pid reuse) is 0, not a huge number.
        assert_eq!(cpu_percent(900, 100, Duration::from_secs(1)), 0.0);
    }

    #[test]
    fn first_sample_has_no_cpu_second_has() {
        let raw = |cpu_ns| Raw {
            pid: 7,
            name: "Zen".into(),
            memory: 1,
            cpu_ns,
            unresponsive: false,
            windows: 0,
        };
        let start = Instant::now();
        let mut sampler = Sampler::default();
        assert_eq!(sampler.rows(vec![raw(0)], start)[0].cpu, None);
        let later = start + Duration::from_secs(1);
        let cpu = sampler.rows(vec![raw(250_000_000)], later)[0].cpu;
        assert!((cpu.unwrap_or(-1.0) - 25.0).abs() < 0.01);
    }

    #[test]
    fn filter_ignores_case_and_edges() {
        let rows = vec![row("Telegram", 1, false), row("Zen", 2, false)];
        assert_eq!(filter_rows(&rows, " TELE ").len(), 1);
        assert_eq!(filter_rows(&rows, "").len(), 2);
        assert!(filter_rows(&rows, "xyz").is_empty());
    }

    #[test]
    fn memory_reads_like_activity_monitor() {
        assert_eq!(format_memory(380 * 1024 * 1024), "380 MB");
        assert_eq!(format_memory(2_040_109_466), "1.9 GB");
        assert_eq!(format_memory(512 * 1024), "512 KB");
    }
}
