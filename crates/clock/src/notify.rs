//! Telling the user a timer ran out or an alarm rang (Linux; Telmo.app does
//! this on macOS).
//!
//! Nix bakes in the store paths of `notify-send` (`TELMO_NOTIFY_SEND`), of a
//! sound player (`TELMO_PW_PLAY`) and of a freedesktop sound (`TELMO_SOUND_FILE`),
//! because a systemd user timer starts us with a bare PATH. Without them we
//! fall back to PATH, and with no sound player or file the alert is silent.

use crate::model::{Fired, Kind};

/// The notification's title and body.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn message(fired: &Fired) -> (String, String) {
    let named = |fallback: String| {
        if fired.name.is_empty() {
            fallback
        } else {
            format!("{} · {}", fired.name, fired.detail)
        }
    };
    match (fired.kind, fired.missed) {
        (Kind::Timer, _) => (
            "Timer done".into(),
            named(format!("{} timer", fired.detail)),
        ),
        (Kind::Alarm, false) => ("Alarm".into(), named(fired.detail.clone())),
        (Kind::Alarm, true) => ("Missed alarm".into(), named(fired.detail.clone())),
    }
}

/// Arguments for `notify-send`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn notify_send_args(fired: &Fired) -> Vec<String> {
    let (title, body) = message(fired);
    vec![
        "--app-name=telmo".into(),
        "--icon=appointment-soon".into(),
        "--urgency=critical".into(),
        title,
        body,
    ]
}

/// Whether to make a sound: not for an alarm slept through.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn plays_sound(fired: &Fired) -> bool {
    !fired.missed
}

#[cfg(target_os = "linux")]
pub fn announce(fired: &Fired) {
    use std::process::{Command, Stdio};

    let notify_send = option_env!("TELMO_NOTIFY_SEND").unwrap_or("notify-send");
    let _ = Command::new(notify_send)
        .args(notify_send_args(fired))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let Some(file) = option_env!("TELMO_SOUND_FILE").filter(|f| std::path::Path::new(f).is_file())
    else {
        return;
    };
    if !plays_sound(fired) {
        return;
    }
    let players = [option_env!("TELMO_PW_PLAY").unwrap_or("pw-play"), "paplay"];
    for player in players {
        let played = Command::new(player)
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if played.is_ok_and(|status| status.success()) {
            return;
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn announce(_fired: &Fired) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fired(kind: Kind, name: &str, detail: &str, missed: bool) -> Fired {
        Fired {
            kind,
            name: name.into(),
            detail: detail.into(),
            missed,
        }
    }

    #[test]
    fn messages() {
        let tea = fired(Kind::Timer, "tea", "5m", false);
        assert_eq!(message(&tea), ("Timer done".into(), "tea · 5m".into()));
        let plain = fired(Kind::Timer, "", "25m", false);
        assert_eq!(message(&plain), ("Timer done".into(), "25m timer".into()));
        let wake = fired(Kind::Alarm, "Wake up", "07:30", false);
        assert_eq!(message(&wake), ("Alarm".into(), "Wake up · 07:30".into()));
        let unnamed = fired(Kind::Alarm, "", "07:30", false);
        assert_eq!(message(&unnamed).1, "07:30");
        let missed = fired(Kind::Alarm, "Wake up", "07:30", true);
        assert_eq!(message(&missed).0, "Missed alarm");
        assert!(plays_sound(&wake) && !plays_sound(&missed));
    }

    #[test]
    fn notify_send_arguments() {
        let args = notify_send_args(&fired(Kind::Timer, "tea", "5m", false));
        assert_eq!(args[0], "--app-name=telmo");
        assert_eq!(args[args.len() - 2..], ["Timer done", "tea · 5m"]);
    }
}
