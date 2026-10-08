//! What the popup knows about the rebuild: the last status it saw, and for
//! how long to show the success line. Time is the app's clock (seconds), so
//! tests drive it.

use crate::rebuild::{self, State, Status};

/// How long the success line replaces the keys.
const BANNER_SECONDS: f32 = 4.0;
/// How long to wait for a started runner to show up in the state file.
const START_TIMEOUT: f32 = 15.0;
const DID_NOT_START: &str = "The rebuild didn't start. Press L to see the log.";

pub enum Change {
    Finished,
    Failed(String),
}

struct Seen {
    status: Status,
    /// App clock when the status was seen.
    at: f32,
    /// Seconds the run had been going at `at`.
    elapsed: u64,
}

pub struct Tracker {
    host: String,
    seen: Option<Seen>,
    /// Started from here, and the runner hasn't written its state yet.
    launched: Option<f32>,
    banner: Option<(String, f32)>,
}

impl Tracker {
    pub fn new(host: String) -> Self {
        Self {
            host,
            seen: None,
            launched: None,
            banner: None,
        }
    }

    pub fn running(&self) -> bool {
        self.seen
            .as_ref()
            .is_some_and(|s| s.status.state == State::Running)
    }

    /// Shows progress right away, before the runner has written anything.
    pub fn launch(&mut self, now: f32, unix: u64, waiting: &str) {
        self.launched = Some(now);
        self.seen = Some(Seen {
            status: Status::running(0, unix, waiting),
            at: now,
            elapsed: 0,
        });
    }

    /// Takes a status read from the state file (or made up by `--mock`).
    pub fn apply(&mut self, status: Status, unix: u64, now: f32) -> Option<Change> {
        if let Some(launched) = self.launched {
            let older = self
                .seen
                .as_ref()
                .is_some_and(|s| status.started < s.status.started);
            if older {
                // Still the previous run's file.
                if now - launched < START_TIMEOUT {
                    return None;
                }
                self.launched = None;
                self.seen = None;
                return Some(Change::Failed(DID_NOT_START.into()));
            }
            self.launched = None;
        }
        let was_running = self.running();
        let elapsed = status
            .finished
            .unwrap_or(unix)
            .saturating_sub(status.started);
        let state = status.state;
        let error = status.error.clone();
        if was_running && state == State::Ok {
            let text = format!(
                "✓ {} · {}",
                rebuild::summary(&status, &self.host),
                clock(elapsed)
            );
            self.banner = Some((text, now + BANNER_SECONDS));
        }
        self.seen = Some(Seen {
            status,
            at: now,
            elapsed,
        });
        match (was_running, state) {
            (true, State::Ok) => Some(Change::Finished),
            (true, State::Failed) => Some(Change::Failed(error.unwrap_or_default())),
            _ => None,
        }
    }

    /// The running status and its elapsed seconds.
    pub fn progress(&self, now: f32) -> Option<(&Status, u64)> {
        let seen = self.seen.as_ref().filter(|_| self.running())?;
        let extra = (now - seen.at).max(0.0) as u64;
        Some((&seen.status, seen.elapsed + extra))
    }

    /// The success line, while it is due.
    pub fn banner(&self, now: f32) -> Option<&str> {
        self.banner
            .as_ref()
            .filter(|(_, until)| now < *until)
            .map(|(text, _)| text.as_str())
    }
}

/// `m:ss`.
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> Tracker {
        Tracker::new("swift".into())
    }

    fn status(state: State, started: u64) -> Status {
        let mut s = Status::running(1, started, "");
        s.state = state;
        s
    }

    #[test]
    fn running_to_ok_finishes_and_shows_the_banner() {
        let mut t = tracker();
        assert!(t.apply(status(State::Running, 100), 105, 0.0).is_none());
        assert!(t.running());
        let mut done = status(State::Ok, 100);
        done.finished = Some(285);
        done.generation = Some(279);
        assert!(matches!(t.apply(done, 285, 1.0), Some(Change::Finished)));
        assert!(!t.running());
        assert_eq!(t.banner(2.0), Some("✓ swift is on generation 279 · 3:05"));
        assert_eq!(t.banner(5.5), None);
    }

    #[test]
    fn running_to_failed_reports_the_sentence() {
        let mut t = tracker();
        t.apply(status(State::Running, 100), 100, 0.0);
        let mut failed = status(State::Failed, 100);
        failed.error = Some("nope".into());
        assert!(matches!(t.apply(failed, 101, 1.0), Some(Change::Failed(m)) if m == "nope"));
    }

    #[test]
    fn an_old_finished_run_is_not_news() {
        let mut t = tracker();
        assert!(t.apply(status(State::Ok, 100), 500, 0.0).is_none());
        assert_eq!(t.banner(0.0), None);
    }

    #[test]
    fn elapsed_keeps_counting_between_reads() {
        let mut t = tracker();
        t.apply(status(State::Running, 100), 141, 10.0);
        assert_eq!(t.progress(10.0).map(|p| p.1), Some(41));
        assert_eq!(t.progress(13.4).map(|p| p.1), Some(44));
    }

    #[test]
    fn launch_ignores_the_previous_run_until_the_runner_writes() {
        let mut t = tracker();
        t.launch(0.0, 1000, "waiting for Touch ID…");
        assert!(t.running());
        // The previous run's file.
        assert!(t.apply(status(State::Ok, 500), 1001, 1.0).is_none());
        assert!(t.running());
        // The new run.
        assert!(t.apply(status(State::Running, 1000), 1002, 2.0).is_none());
        assert!(t.running());
    }

    #[test]
    fn a_runner_that_never_writes_fails_after_a_while() {
        let mut t = tracker();
        t.launch(0.0, 1000, "waiting");
        let change = t.apply(status(State::Ok, 500), 1020, 16.0);
        assert!(matches!(change, Some(Change::Failed(m)) if m == DID_NOT_START));
        assert!(!t.running());
    }
}
