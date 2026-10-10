//! The only writer of `clock.json`. Everything that changes it (the popup,
//! `timer start`, `fire`) goes through `apply`, which holds the file's lock for
//! the read-change-write and then updates the wake-up schedule to match.
//! Telmo.app only reads the file.

use crate::{
    model::ClockState,
    sched::{self, Action},
};
use std::{io, path::Path};
use telmo_kit::state;

pub const NAME: &str = "clock";

pub fn load() -> ClockState {
    state::load(NAME).unwrap_or_default()
}

/// What a change produced: the new state, the change's own result and any
/// trouble arming wake-ups (one line each).
pub struct Applied<R> {
    pub state: ClockState,
    pub result: R,
    pub problems: Vec<String>,
}

pub fn apply<R>(now_ms: i64, change: impl FnOnce(&mut ClockState) -> R) -> io::Result<Applied<R>> {
    let dir = state::dir().ok_or_else(|| io::Error::other("no home directory"))?;
    let (state, result, actions) = locked(&dir, change)?;
    let problems = sched::run(&actions, now_ms);
    Ok(Applied {
        state,
        result,
        problems,
    })
}

/// The locked read-change-write, and the schedule changes it implies.
pub fn locked<R>(
    dir: &Path,
    change: impl FnOnce(&mut ClockState) -> R,
) -> io::Result<(ClockState, R, Vec<Action>)> {
    let mut before = None;
    let mut result = None;
    let after = state::update_in(dir, NAME, |state: &mut ClockState| {
        before = Some(state.clone());
        result = Some(change(state));
    })?;
    let actions = sched::plan(&before.unwrap_or_default(), &after);
    let result = result.ok_or_else(|| io::Error::other("the change did not run"))?;
    Ok((after, result, actions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_from_several_threads_all_land_and_plan_the_schedule() {
        let dir = std::env::temp_dir().join(format!("telmo-clock-store-{}", std::process::id()));
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        let now = 1_790_000_000_000 + (worker * 100 + i) as i64;
                        let (_, id, actions) =
                            locked(&dir, |s| s.add_timer("", 60_000, now)).expect("locked");
                        assert_eq!(actions.len(), 1);
                        assert!(matches!(&actions[0], Action::Schedule { id: a, .. } if *a == id));
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("worker");
        }
        let (state, count, actions) = locked(&dir, |s| s.timers.len()).expect("read");
        assert_eq!(count, 40);
        assert_eq!(state.timers.len(), 40);
        assert!(actions.is_empty());
        let ids: std::collections::HashSet<_> = state.timers.iter().map(|t| &t.id).collect();
        assert_eq!(ids.len(), 40, "ids stay unique");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_json_starts_over() {
        let dir = std::env::temp_dir().join(format!("telmo-clock-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("clock.json"), "not json").expect("write");
        let (state, ..) = locked(&dir, |s| s.add_timer("", 1000, 5)).expect("locked");
        assert_eq!(state.timers.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
