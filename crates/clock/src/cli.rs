//! The subcommands: `telmo-clock timer start|list`, `fire`, `fired`, `sync`.
//!
//! * `fire <id>` is what a Linux systemd timer runs: announce, then mark fired.
//! * `fired <id>` only marks (Telmo.app on macOS has already announced).
//!
//! Both do nothing if the item is not really due (paused, deleted or moved
//! since the wake-up was armed), so a stale timer is harmless.

use crate::{
    fmt,
    model::{Fired, TimerStatus},
    notify, parse, sched, store,
};
use jiff::tz::TimeZone;

pub const USAGE: &str = "usage:
  telmo-clock                          open the popup
  telmo-clock timer start <length> [--name NAME]
  telmo-clock timer list
  telmo-clock fire <id>                announce a due timer or alarm (Linux, run by systemd)
  telmo-clock fired <id>               mark a due timer or alarm announced (Telmo.app)
  telmo-clock sync                     arm wake-ups for everything pending (Linux)";

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Whether `args` (after the program name) is a subcommand rather than the popup.
pub fn is_command(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("timer" | "fire" | "fired" | "sync")
    )
}

/// Runs a subcommand and returns what to print, or the error.
pub fn run(args: &[String]) -> Result<String, String> {
    let now = now_ms();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["timer", "start", ..] => timer_start(&args[2..], now),
        ["timer", "list"] => Ok(timer_list(&store::load(), now)),
        ["fire", id] => mark(id, now, true),
        ["fired", id] => mark(id, now, false),
        ["sync"] => sync(now),
        _ => Err(USAGE.into()),
    }
}

fn timer_start(args: &[String], now: i64) -> Result<String, String> {
    let (length, name) = start_arguments(args)?;
    let duration = parse::duration(&length).map_err(|e| e.to_string())?;
    let applied = store::apply(now, |s| s.add_timer(&name, duration, now))
        .map_err(|e| format!("Can't save the timer: {e}"))?;
    let mut line = format!("Started a {} timer", fmt::short(duration));
    if !name.is_empty() {
        line += &format!(" ({name})");
    }
    line += &format!(" [{}]", applied.result);
    if let Some(problem) = applied.problems.first() {
        line += &format!("\nWarning: {problem}");
    }
    Ok(line)
}

/// `<length> [--name NAME | -n NAME | NAME words…]`.
fn start_arguments(args: &[String]) -> Result<(String, String), String> {
    let mut length = None;
    let mut name = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--name" | "-n" => {
                let value = rest.next().ok_or("--name needs a value")?;
                name.push(value.clone());
            }
            other if other.starts_with("--name=") => {
                name.push(other["--name=".len()..].to_string())
            }
            _ if length.is_none() => length = Some(arg.clone()),
            _ => name.push(arg.clone()),
        }
    }
    let length = length.ok_or("usage: telmo-clock timer start <length> [--name NAME]")?;
    Ok((length, name.join(" ")))
}

pub fn timer_list(state: &crate::model::ClockState, now: i64) -> String {
    if state.timers.is_empty() {
        return "No timers.".into();
    }
    state
        .timers
        .iter()
        .map(|t| {
            let (status, left) = match t.status(now) {
                TimerStatus::Running => ("running", fmt::countdown(t.remaining(now))),
                TimerStatus::Paused => ("paused", fmt::countdown(t.remaining(now))),
                TimerStatus::Done => ("done", "0:00".into()),
            };
            format!(
                "{}  {status:<8} {left:>8}  {:>6}  {}",
                t.id,
                fmt::short(t.duration_ms),
                t.name
            )
            .trim_end()
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mark(id: &str, now: i64, announce: bool) -> Result<String, String> {
    if announce && !cfg!(target_os = "linux") {
        return Err("`fire` is for Linux; on macOS Telmo.app announces and runs `fired`.".into());
    }
    let tz = TimeZone::system();
    let applied = store::apply(now, |s| s.mark_fired(id, now, &tz))
        .map_err(|e| format!("Can't update the clock: {e}"))?;
    let fired: Option<Fired> = applied.result;
    if let (true, Some(fired)) = (announce, &fired) {
        notify::announce(fired);
    }
    Ok(String::new())
}

fn sync(now: i64) -> Result<String, String> {
    let state = store::load();
    let actions = sched::plan_all(&state);
    let problems = sched::run(&actions, now);
    match problems.first() {
        Some(problem) => Err(problem.clone()),
        None => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn start_arguments_in_any_order() {
        assert_eq!(
            start_arguments(&words(&["5m"])),
            Ok(("5m".into(), String::new()))
        );
        assert_eq!(
            start_arguments(&words(&["5m", "--name", "tea time"])),
            Ok(("5m".into(), "tea time".into()))
        );
        assert_eq!(
            start_arguments(&words(&["--name=tea", "90s"])),
            Ok(("90s".into(), "tea".into()))
        );
        assert_eq!(
            start_arguments(&words(&["10m", "pasta", "water"])),
            Ok(("10m".into(), "pasta water".into()))
        );
        assert!(start_arguments(&words(&[])).is_err());
        assert!(start_arguments(&words(&["5m", "--name"])).is_err());
    }

    #[test]
    fn list_is_plain_text() {
        let mut state = crate::model::ClockState::default();
        assert_eq!(timer_list(&state, 0), "No timers.");
        let id = state.add_timer("tea", 300_000, 1_000);
        assert_eq!(
            timer_list(&state, 1_000),
            format!("{id}  running      5:00      5m  tea")
        );
        state.timers[0].pause(61_000);
        assert!(timer_list(&state, 99_000).contains("paused       4:00"));
    }

    #[test]
    fn commands_are_recognised() {
        assert!(is_command(&words(&["timer", "list"])));
        assert!(is_command(&words(&["fire", "abc"])));
        assert!(!is_command(&words(&["--mock"])));
        assert!(!is_command(&[]));
    }
}
