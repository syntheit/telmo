//! macOS backend: `pmset` and `ioreg` for the battery, `osascript` to ask for
//! the administrator password when changing Low Power Mode, `top` for the
//! energy users. Polls every 10 s; the popup is only open briefly.

use super::{AWAKE, Cmd, Event, MODE, Rx, Tx, awake};
use crate::model::{Battery, ChargeState, EnergyUser, Health, ModeSetting, PowerMode, Snapshot};
use std::{process::Stdio, time::Duration};
use tokio::{process::Command, task::JoinHandle, time::interval};

const POLL: Duration = Duration::from_secs(10);

pub fn spawn(cmds: Rx, events: Tx) -> JoinHandle<()> {
    tokio::spawn(run(cmds, events))
}

async fn run(mut cmds: Rx, events: Tx) {
    let users = events.clone();
    tokio::spawn(async move {
        let _ = users.send(Event::EnergyUsers(energy_users().await));
    });
    let mut last = None;
    let mut poll = interval(POLL);
    loop {
        tokio::select! {
            _ = poll.tick() => publish(&events, &mut last).await,
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                let (target, result) = handle(cmd).await;
                let _ = events.send(Event::Done { target, result });
                publish(&events, &mut last).await;
            }
        }
    }
}

async fn publish(events: &Tx, last: &mut Option<Snapshot>) {
    let snapshot = snapshot().await;
    if last.as_ref() != Some(&snapshot) {
        let _ = events.send(Event::Snapshot(snapshot.clone()));
        *last = Some(snapshot);
    }
}

async fn handle(cmd: Cmd) -> (&'static str, Result<String, String>) {
    match cmd {
        Cmd::SetMode(mode) => (MODE, set_low_power(mode == PowerMode::Saver).await),
        Cmd::SetKeepAwake(choice) => (AWAKE, awake::set(choice).await),
    }
}

async fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn snapshot() -> Snapshot {
    let (pmset, batt, ioreg) = tokio::join!(
        output("pmset", &["-g"]),
        output("pmset", &["-g", "batt"]),
        output("ioreg", &["-rn", "AppleSmartBattery"]),
    );
    let low_power = pmset.as_deref().and_then(parse_low_power).unwrap_or(false);
    let mode = if low_power {
        PowerMode::Saver
    } else {
        PowerMode::Balanced
    };
    Snapshot {
        battery: batt
            .as_deref()
            .and_then(|batt| parse_battery(batt, ioreg.as_deref().unwrap_or(""))),
        mode: Some(ModeSetting {
            current: mode,
            available: vec![PowerMode::Balanced, PowerMode::Saver],
        }),
        keep_awake: awake::current().await,
        lists_energy_users: true,
    }
}

/// `pmset -g` has a line like " lowpowermode         0".
fn parse_low_power(pmset: &str) -> Option<bool> {
    pmset.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == "lowpowermode").then(|| words.next() == Some("1"))
    })
}

/// `pmset -g batt`:
/// ```text
/// Now drawing from 'AC Power'
///  -InternalBattery-0 (id=1234)    82%; charging; 1:05 remaining present: true
/// ```
fn parse_battery(batt: &str, ioreg: &str) -> Option<Battery> {
    let line = batt.lines().find(|l| l.contains("InternalBattery"))?;
    let detail = line.split('\t').nth(1)?;
    let mut parts = detail.split(';').map(str::trim);
    let percent: u8 = parts.next()?.strip_suffix('%')?.parse().ok()?;
    let status = parts.next()?;
    let rest = parts.next().unwrap_or("");
    let plugged = batt.contains("'AC Power'");
    let state = match status {
        "charging" => ChargeState::Charging,
        "discharging" => ChargeState::OnBattery,
        "charged" | "finishing charge" => ChargeState::Full,
        _ if plugged => ChargeState::NotCharging,
        _ => ChargeState::OnBattery,
    };
    let minutes = rest.split_whitespace().next().and_then(parse_clock);
    let watts = number_after(ioreg, "Watts");
    Some(Battery {
        percent,
        state,
        minutes: minutes.filter(|_| state != ChargeState::Full),
        source: plugged.then(|| match watts {
            Some(w) => format!("{w} W adapter"),
            None => "power adapter".into(),
        }),
        health: parse_health(ioreg),
    })
}

/// "4:12" is minutes in H:MM form; "(no" (estimate) is not a time.
fn parse_clock(text: &str) -> Option<u32> {
    let (hours, minutes) = text.split_once(':')?;
    Some(hours.parse::<u32>().ok()? * 60 + minutes.parse::<u32>().ok()?)
}

/// The integer after `"key"`, in either `"CycleCount" = 214` (ioreg lines)
/// or `"DesignCapacity"=5760` (inside ioreg's inline dictionaries).
fn number_after(text: &str, key: &str) -> Option<u32> {
    let quoted = format!("\"{key}\"");
    text.match_indices(&quoted).find_map(|(at, _)| {
        let rest = text[at + quoted.len()..].trim_start().strip_prefix('=')?;
        let digits: String = rest
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

fn parse_health(ioreg: &str) -> Option<Health> {
    let cycles = number_after(ioreg, "CycleCount");
    let max = number_after(ioreg, "NominalChargeCapacity")
        .or_else(|| number_after(ioreg, "AppleRawMaxCapacity"));
    let design = number_after(ioreg, "DesignCapacity").filter(|d| *d > 0);
    let max_capacity = max
        .zip(design)
        .map(|(max, design)| ((max * 100 + design / 2) / design).min(100) as u8);
    if cycles.is_none() && max_capacity.is_none() {
        return None;
    }
    // macOS calls anything under 80% "Service Recommended".
    let condition = max_capacity.map(|c| {
        if c < 80 {
            "service recommended"
        } else {
            "normal"
        }
    });
    Some(Health {
        max_capacity,
        cycles,
        condition: condition.map(str::to_string),
    })
}

/// Low Power Mode is a system setting, so changing it needs administrator
/// rights: ask for them with the usual macOS password dialog.
async fn set_low_power(on: bool) -> Result<String, String> {
    let script = admin_script(&format!("/usr/bin/pmset -a lowpowermode {}", u8::from(on)));
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("Could not run osascript: {e}"))?;
    if output.status.success() {
        let state = if on { "on" } else { "off" };
        return Ok(format!("Low Power Mode is {state}."));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("-128") {
        return Err("Cancelled. Low Power Mode was not changed.".into());
    }
    Err(format!(
        "Could not change Low Power Mode: {}",
        stderr.trim()
    ))
}

/// AppleScript for running a shell command as administrator.
fn admin_script(command: &str) -> String {
    format!(
        "do shell script {} with administrator privileges",
        applescript_quote(command)
    )
}

fn applescript_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Five apps using the most energy; the second `top` sample is the real one.
async fn energy_users() -> Vec<EnergyUser> {
    let args = [
        "-l",
        "2",
        "-s",
        "1",
        "-o",
        "power",
        "-stats",
        "command,power",
        "-n",
        "5",
    ];
    let text = output("top", &args).await.unwrap_or_default();
    parse_top(&text)
}

fn parse_top(text: &str) -> Vec<EnergyUser> {
    let after_last_header = text.rsplit("COMMAND").next().unwrap_or("");
    after_last_header
        .lines()
        .filter_map(|line| {
            let (name, power) = line.trim_end().rsplit_once(' ')?;
            let name = name.trim().trim_end_matches('(').trim();
            let power: f32 = power.parse().ok()?;
            (!name.is_empty()).then(|| EnergyUser {
                name: name.to_string(),
                power,
            })
        })
        .take(5)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BATT: &str = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=24576099)\t82%; charging; 1:05 remaining present: true\n";
    const IOREG: &str = "    | |   \"AdapterDetails\" = {\"Watts\"=96,\"Voltage\"=20000}\n    | |   \"CycleCount\" = 214\n    | |   \"BatteryData\" = {\"NominalChargeCapacity\"=5690,\"DesignCapacity\"=6249}\n";

    #[test]
    fn reads_battery() {
        let battery = parse_battery(BATT, IOREG).expect("battery");
        assert_eq!(battery.percent, 82);
        assert_eq!(battery.state, ChargeState::Charging);
        assert_eq!(battery.minutes, Some(65));
        assert_eq!(battery.source.as_deref(), Some("96 W adapter"));
        let health = battery.health.expect("health");
        assert_eq!(health.max_capacity, Some(91));
        assert_eq!(health.cycles, Some(214));
        assert_eq!(health.condition.as_deref(), Some("normal"));
    }

    #[test]
    fn desktop_has_no_battery() {
        assert!(parse_battery("Now drawing from 'AC Power'\n", "").is_none());
    }

    #[test]
    fn on_battery_without_estimate() {
        let batt = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t96%; discharging; (no estimate) present: true\n";
        let battery = parse_battery(batt, "").expect("battery");
        assert_eq!(battery.state, ChargeState::OnBattery);
        assert_eq!(battery.minutes, None);
        assert_eq!(battery.source, None);
    }

    #[test]
    fn low_power_line() {
        assert_eq!(
            parse_low_power(" sleep 1\n lowpowermode         1\n"),
            Some(true)
        );
        assert_eq!(parse_low_power(" lowpowermode 0\n"), Some(false));
        assert_eq!(parse_low_power(" sleep 1\n"), None);
    }

    #[test]
    fn quotes_for_applescript() {
        assert_eq!(applescript_quote(r#"a "b" \c"#), r#""a \"b\" \\c""#);
    }

    #[test]
    fn top_uses_last_sample() {
        let text = "COMMAND          POWER\nOld  1.0\n\nProcesses: 1\nCOMMAND          POWER\nZen              46.8 \nSpotify Helper ( 31.5 \nGhostty          12.1\n";
        let users = parse_top(text);
        assert_eq!(users.len(), 3);
        assert_eq!(users[1].name, "Spotify Helper");
        assert_eq!(users[0].power, 46.8);
    }
}
