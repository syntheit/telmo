//! Waking `telmo-clock fire <id>` when a timer or alarm is due.
//!
//! macOS: nothing to do here; Telmo.app watches `clock.json` itself.
//!
//! Linux: every pending item gets a transient user timer
//! (`systemd-run --user --on-calendar=…`), made when the item is created or
//! changed and stopped when it is paused or deleted. The timers are not kept
//! across a reboot, so `telmo-clock sync` (run at login by the Home Manager
//! module) makes them again.

use crate::model::ClockState;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Cancel(String),
    Schedule { id: String, due_ms: i64 },
}

fn due_map(state: &ClockState) -> BTreeMap<String, i64> {
    state.due().into_iter().collect()
}

/// What to undo and what to arm so the schedule matches `after`.
pub fn plan(before: &ClockState, after: &ClockState) -> Vec<Action> {
    let (old, new) = (due_map(before), due_map(after));
    let mut actions = Vec::new();
    for id in old.keys() {
        if old.get(id) != new.get(id) {
            actions.push(Action::Cancel(id.clone()));
        }
    }
    for (id, &due_ms) in &new {
        if old.get(id) != Some(&due_ms) {
            actions.push(Action::Schedule {
                id: id.clone(),
                due_ms,
            });
        }
    }
    actions
}

/// Everything pending, from scratch (stale timers are stopped first).
pub fn plan_all(state: &ClockState) -> Vec<Action> {
    let mut actions = Vec::new();
    for (id, due_ms) in due_map(state) {
        actions.push(Action::Cancel(id.clone()));
        actions.push(Action::Schedule { id, due_ms });
    }
    actions
}

/// The due time is part of the name: a repeating alarm arms its next ring
/// while the unit of this one may still be finishing, and the two must not clash.
pub fn unit(id: &str, due_ms: i64) -> String {
    format!("telmo-clock-{id}-{}", due_ms.div_euclid(1000))
}

/// Arguments for `systemctl`: stop every wake-up this item has.
pub fn stop_args(id: &str) -> Vec<String> {
    vec![
        "--user".into(),
        "stop".into(),
        format!("telmo-clock-{id}-*.timer"),
    ]
}

/// `2026-10-10 17:07:00 UTC`, rounded up to the next whole second.
pub fn on_calendar(due_ms: i64) -> String {
    let secs = (due_ms + 999).div_euclid(1000);
    match jiff::Timestamp::from_second(secs) {
        Ok(ts) => ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
        Err(_) => "now".into(),
    }
}

/// Arguments for `systemd-run`. Something already due (a machine that was
/// off) fires a second from now instead, as a calendar time in the past
/// would never come.
pub fn run_args(exe: &str, id: &str, due_ms: i64, now_ms: i64) -> Vec<String> {
    let when = if due_ms <= now_ms {
        "--on-active=1s".to_string()
    } else {
        format!("--on-calendar={}", on_calendar(due_ms))
    };
    vec![
        "--user".into(),
        format!("--unit={}", unit(id, due_ms)),
        "--description=telmo clock".into(),
        when,
        "--timer-property=AccuracySec=1s".into(),
        "--collect".into(),
        "--quiet".into(),
        exe.into(),
        "fire".into(),
        id.into(),
    ]
}

/// Where `program` is: on `path` (a `PATH` value), else in the usual system
/// places. A systemd user unit (`telmo-clock-sync` at login, a `fire` run by
/// a timer) can start us with a PATH that has neither `systemctl` nor
/// `systemd-run`.
pub fn locate(
    program: &str,
    path: Option<&std::ffi::OsStr>,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    const SYSTEM: [&str; 3] = ["/run/current-system/sw/bin", "/usr/bin", "/bin"];
    let from_path = path.into_iter().flat_map(std::env::split_paths);
    from_path
        .chain(SYSTEM.iter().map(PathBuf::from))
        .map(|dir| dir.join(program))
        .find(|candidate| exists(candidate))
        .unwrap_or_else(|| PathBuf::from(program))
}

/// Carries the plan out. Only Linux has anything to run; elsewhere this does
/// nothing. Returns the problems met, one line each.
pub fn run(actions: &[Action], now_ms: i64) -> Vec<String> {
    if !cfg!(target_os = "linux") || actions.is_empty() {
        return Vec::new();
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe.to_string_lossy().into_owned(),
        Err(e) => return vec![format!("Can't find telmo-clock to schedule: {e}")],
    };
    let mut problems = Vec::new();
    for action in actions {
        let (program, args) = match action {
            Action::Cancel(id) => ("systemctl", stop_args(id)),
            Action::Schedule { id, due_ms } => ("systemd-run", run_args(&exe, id, *due_ms, now_ms)),
        };
        let program = locate(program, std::env::var_os("PATH").as_deref(), |p| {
            p.is_file()
        });
        let result = std::process::Command::new(program)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match (action, result) {
            // Stopping a timer that is not there is fine.
            (Action::Cancel(_), _) => {}
            (_, Ok(status)) if status.success() => {}
            (_, Ok(status)) => problems.push(format!("systemd-run failed ({status})")),
            (_, Err(e)) => problems.push(format!("Can't run systemd-run: {e}")),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Timer;

    const T0: i64 = 1_790_000_000_000;

    #[test]
    fn plan_follows_changes() {
        let mut state = ClockState::default();
        let a = state.add_timer("a", 60_000, T0);
        let b = state.add_timer("b", 120_000, T0);
        let before = state.clone();

        assert_eq!(
            plan(&ClockState::default(), &before).len(),
            2,
            "new timers are armed"
        );
        assert!(plan(&before, &before).is_empty(), "no change, no work");

        // Pausing cancels; resuming arms again.
        let mut paused = before.clone();
        paused.timers[0].pause(T0 + 1000);
        assert_eq!(plan(&before, &paused), vec![Action::Cancel(a.clone())]);
        let mut resumed = paused.clone();
        resumed.timers[0].resume(T0 + 5000);
        assert_eq!(
            plan(&paused, &resumed),
            vec![Action::Schedule {
                id: a.clone(),
                due_ms: T0 + 5000 + 59_000
            }]
        );

        // Restarting moves it: cancel, then arm at the new time.
        let mut restarted = before.clone();
        restarted.timers[1].restart(T0 + 10_000);
        assert_eq!(
            plan(&before, &restarted),
            vec![
                Action::Cancel(b.clone()),
                Action::Schedule {
                    id: b.clone(),
                    due_ms: T0 + 130_000
                }
            ]
        );

        // Deleting cancels; a fired timer needs no wake-up.
        let mut deleted = before.clone();
        deleted.timers.remove(0);
        assert_eq!(plan(&before, &deleted), vec![Action::Cancel(a)]);
        let mut fired = before.clone();
        fired.timers[1] = Timer {
            fired: true,
            ..fired.timers[1].clone()
        };
        assert_eq!(plan(&before, &fired), vec![Action::Cancel(b)]);
    }

    #[test]
    fn sync_rearms_everything() {
        let mut state = ClockState::default();
        let a = state.add_timer("a", 60_000, T0);
        assert_eq!(
            plan_all(&state),
            vec![
                Action::Cancel(a.clone()),
                Action::Schedule {
                    id: a,
                    due_ms: T0 + 60_000
                }
            ]
        );
    }

    #[test]
    fn systemd_tools_are_found_even_with_a_bare_path() {
        let has = |present: &'static str| move |p: &Path| p == Path::new(present);
        let bare = std::ffi::OsStr::new("/nix/store/x/bin");
        assert_eq!(
            locate(
                "systemctl",
                Some(bare),
                has("/run/current-system/sw/bin/systemctl")
            ),
            Path::new("/run/current-system/sw/bin/systemctl")
        );
        assert_eq!(
            locate("systemctl", Some(bare), has("/nix/store/x/bin/systemctl")),
            Path::new("/nix/store/x/bin/systemctl"),
            "PATH comes first"
        );
        assert_eq!(
            locate("systemd-run", None, |_| false),
            Path::new("systemd-run"),
            "nothing found: the bare name, so the error names the program"
        );
    }

    #[test]
    fn systemd_arguments() {
        // 1_790_000_000 is 2026-09-21 14:13:20 UTC.
        assert_eq!(on_calendar(1_790_000_000_000), "2026-09-21 14:13:20 UTC");
        assert_eq!(
            on_calendar(1_790_000_000_001),
            "2026-09-21 14:13:21 UTC",
            "rounds up"
        );
        assert_eq!(
            run_args(
                "/nix/store/x/bin/telmo-clock",
                "00ab12cd",
                1_790_000_000_000,
                1_789_999_000_000
            ),
            [
                "--user",
                "--unit=telmo-clock-00ab12cd-1790000000",
                "--description=telmo clock",
                "--on-calendar=2026-09-21 14:13:20 UTC",
                "--timer-property=AccuracySec=1s",
                "--collect",
                "--quiet",
                "/nix/store/x/bin/telmo-clock",
                "fire",
                "00ab12cd",
            ]
        );
        assert!(
            run_args("x", "i", 5, 10).contains(&"--on-active=1s".to_string()),
            "overdue fires now"
        );
        assert_eq!(
            stop_args("00ab12cd"),
            ["--user", "stop", "telmo-clock-00ab12cd-*.timer"]
        );
    }
}
