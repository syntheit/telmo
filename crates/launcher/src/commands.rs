//! The `!` commands: the telmo popups, power actions, and System Settings panes.

use crate::rank::Ranker;
use ratatui::style::Color;
use telmo_kit::theme;

/// The session actions `telmo-system action <name>` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Lock,
    Sleep,
    Restart,
    ShutDown,
    LogOut,
}

impl Power {
    /// The argument `telmo-system action` takes.
    pub fn arg(self) -> &'static str {
        match self {
            Power::Lock => "lock",
            Power::Sleep => "sleep",
            Power::Restart => "restart",
            Power::ShutDown => "shutdown",
            Power::LogOut => "logout",
        }
    }

    /// Restarting, shutting down and logging out ask first.
    pub fn confirms(self) -> bool {
        matches!(self, Power::Restart | Power::ShutDown | Power::LogOut)
    }

    /// `Restart swift?`
    pub fn question(self, host: &str) -> String {
        match self {
            Power::Restart => format!("Restart {host}?"),
            Power::ShutDown => format!("Shut down {host}?"),
            _ => "Log out?".to_string(),
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Power::Lock => "lock",
            Power::Sleep => "sleep",
            Power::Restart => "restart",
            Power::ShutDown => "shut down",
            Power::LogOut => "log out",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `telmo popup <name>`.
    Popup(&'static str),
    Power(Power),
    /// Opens the System popup and starts something there: `rebuild` or `apps`
    /// (Force quit).
    SystemView(&'static str),
    /// An `x-apple.systempreferences:` URL.
    Settings(String),
    /// `telmo-clock timer start <duration> [--name NAME]`.
    Timer {
        duration: String,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub label: String,
    pub glyph: &'static str,
    pub color: Color,
    /// Other words that find it: `!net` finds Wi-Fi.
    pub words: &'static [&'static str],
    pub action: Action,
}

/// What the list depends on in this session.
#[derive(Debug, Clone, Copy, Default)]
pub struct Env {
    pub macos: bool,
    /// `telmo-clock` is installed.
    pub clock: bool,
}

fn command(
    label: &str,
    glyph: &'static str,
    color: Color,
    words: &'static [&'static str],
    action: Action,
) -> Command {
    Command {
        label: label.into(),
        glyph,
        color,
        words,
        action,
    }
}

/// Settings panes worth a direct door: label, pane identifier, search words.
/// The identifiers are the bundle identifiers of the System Settings
/// extensions in `/System/Library/ExtensionKit/Extensions` (macOS 27); a test
/// checks them against the installed system.
pub const PANES: [(&str, &str, &[&str]); 14] = [
    (
        "Network settings",
        "com.apple.Network-Settings.extension",
        &["wifi", "ethernet", "vpn"],
    ),
    (
        "Keyboard settings",
        "com.apple.Keyboard-Settings.extension",
        &["shortcuts", "input"],
    ),
    (
        "Trackpad settings",
        "com.apple.Trackpad-Settings.extension",
        &["gestures", "scroll"],
    ),
    (
        "Mouse settings",
        "com.apple.Mouse-Settings.extension",
        &["pointer"],
    ),
    (
        "Sound settings",
        "com.apple.Sound-Settings.extension",
        &["volume", "audio"],
    ),
    (
        "Displays settings",
        "com.apple.Displays-Settings.extension",
        &["monitor", "resolution"],
    ),
    (
        "Battery settings",
        "com.apple.Battery-Settings.extension",
        &["power", "energy"],
    ),
    (
        "Appearance settings",
        "com.apple.Appearance-Settings.extension",
        &["dark", "theme"],
    ),
    (
        "Wallpaper settings",
        "com.apple.Wallpaper-Settings.extension",
        &["background"],
    ),
    (
        "Notifications settings",
        "com.apple.Notifications-Settings.extension",
        &["alerts"],
    ),
    (
        "Login items settings",
        "com.apple.LoginItems-Settings.extension",
        &["startup", "background"],
    ),
    (
        "Privacy settings",
        "com.apple.settings.PrivacySecurity.extension",
        &["security", "permissions"],
    ),
    (
        "Software update",
        "com.apple.Software-Update-Settings.extension",
        &["upgrade", "macos"],
    ),
    (
        "Storage settings",
        "com.apple.settings.Storage",
        &["disk", "space"],
    ),
];

pub fn pane_url(id: &str) -> String {
    format!("x-apple.systempreferences:{id}")
}

/// Every command, in the order `!` alone lists them.
pub fn all(env: Env) -> Vec<Command> {
    let mut list = vec![
        command(
            "Wi-Fi",
            "\u{f05a9}",
            theme::CYAN,
            &["network", "net", "wifi"],
            Action::Popup("net"),
        ),
        command(
            "Bluetooth",
            "\u{f00af}",
            theme::BLUE,
            &["bt"],
            Action::Popup("bt"),
        ),
        command(
            "Sound",
            "\u{f057e}",
            theme::MAGENTA,
            &["volume", "audio", "eq"],
            Action::Popup("sound"),
        ),
        command(
            "Displays",
            "\u{f0379}",
            theme::FG,
            &["monitor", "brightness"],
            Action::Popup("display"),
        ),
        command(
            "Power",
            "\u{f0079}",
            theme::YELLOW,
            &["battery", "awake"],
            Action::Popup("power"),
        ),
        command(
            "Clipboard",
            "\u{f014c}",
            theme::FG,
            &["paste", "history"],
            Action::Popup("clipboard"),
        ),
        command(
            "System",
            "\u{f056e}",
            theme::BLUE,
            &["overview", "storage"],
            Action::Popup("system"),
        ),
    ];
    if env.clock {
        list.push(command(
            "Clock",
            "\u{f0150}",
            theme::GREEN,
            &["timer", "alarm", "stopwatch"],
            Action::Popup("clock"),
        ));
    }
    list.extend([
        command(
            "Lock",
            "\u{f023}",
            theme::YELLOW,
            &["screen"],
            Action::Power(Power::Lock),
        ),
        command(
            "Sleep",
            "\u{f0904}",
            theme::MAGENTA,
            &["suspend"],
            Action::Power(Power::Sleep),
        ),
        command(
            "Restart",
            "\u{f0709}",
            theme::YELLOW,
            &["reboot"],
            Action::Power(Power::Restart),
        ),
        command(
            "Shut down",
            "\u{23fb}",
            theme::RED,
            &["poweroff", "off"],
            Action::Power(Power::ShutDown),
        ),
        command(
            "Log out",
            "\u{f0343}",
            theme::DIM,
            &["logout", "sign out"],
            Action::Power(Power::LogOut),
        ),
        command(
            "Rebuild",
            "\u{f08ea}",
            theme::BLUE,
            &["nix", "switch", "update"],
            Action::SystemView("rebuild"),
        ),
        command(
            "Force quit",
            "\u{f015c}",
            theme::RED,
            &["kill", "quit", "stuck"],
            Action::SystemView("apps"),
        ),
    ]);
    if env.macos {
        for (label, id, words) in PANES {
            list.push(command(
                label,
                "\u{f0493}",
                theme::DIM,
                words,
                Action::Settings(pane_url(id)),
            ));
        }
    }
    list
}

/// `!t 5m tea` or `!timer 90s`: the duration and the optional name.
pub fn timer_spec(query: &str) -> Option<(String, Option<String>)> {
    let (word, rest) = query.trim_start().split_once(char::is_whitespace)?;
    if !word.eq_ignore_ascii_case("t") && !word.eq_ignore_ascii_case("timer") {
        return None;
    }
    let rest = rest.trim();
    let (duration, name) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if !duration.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let name = name.trim();
    Some((
        duration.to_string(),
        (!name.is_empty()).then(|| name.to_string()),
    ))
}

/// Beats a long label that merely starts the same: `!net` means Wi-Fi, not
/// "Network settings". A short label (`Power`) still wins.
const EXACT_WORD: i64 = 2_900;

/// The commands matching what follows the `!`, best first, with the letters
/// to highlight in the label. Nothing typed lists them all.
pub fn filter(commands: &[Command], query: &str, ranker: &mut Ranker) -> Vec<(usize, Vec<u32>)> {
    let query = query.trim();
    if query.is_empty() {
        return (0..commands.len()).map(|i| (i, Vec::new())).collect();
    }
    let mut found: Vec<(i64, usize, Vec<u32>)> = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let by_label = ranker.text(&command.label, query);
        let by_word = command
            .words
            .iter()
            .filter_map(|w| {
                let score = ranker.text(w, query)?.score;
                // A word typed in full is a name for the command.
                Some(if w.eq_ignore_ascii_case(query) {
                    EXACT_WORD
                } else {
                    score - 200
                })
            })
            .max();
        match (by_label, by_word) {
            (Some(m), word) if word.is_none_or(|w| m.score >= w) => {
                found.push((m.score, index, m.hits));
            }
            (_, Some(score)) => found.push((score, index, Vec::new())),
            _ => {}
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, i, h)| (i, h)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(query: &str, env: Env) -> Vec<String> {
        let list = all(env);
        filter(&list, query, &mut Ranker::new())
            .into_iter()
            .map(|(i, _)| list[i].label.clone())
            .collect()
    }

    const MAC: Env = Env {
        macos: true,
        clock: false,
    };

    #[test]
    fn a_bang_alone_lists_everything_in_order() {
        let out = labels("", MAC);
        assert_eq!(out[0], "Wi-Fi");
        assert_eq!(out.len(), all(MAC).len());
        assert!(out.contains(&"Force quit".to_string()));
    }

    #[test]
    fn sl_finds_sleep_first() {
        assert_eq!(labels("sl", MAC)[0], "Sleep");
        let list = all(MAC);
        let hits = filter(&list, "sl", &mut Ranker::new());
        assert_eq!(hits[0].1, [0, 1]);
    }

    #[test]
    fn other_words_find_commands_without_highlights() {
        assert_eq!(labels("reboot", MAC)[0], "Restart");
        assert_eq!(labels("net", MAC)[0], "Wi-Fi");
        assert!(labels("zzzz", MAC).is_empty());
    }

    #[test]
    fn clock_and_settings_depend_on_the_session() {
        let linux = Env {
            macos: false,
            clock: false,
        };
        assert!(!labels("", linux).iter().any(|l| l.contains("settings")));
        assert!(!labels("", linux).contains(&"Clock".to_string()));
        let with_clock = Env {
            macos: false,
            clock: true,
        };
        assert!(labels("clock", with_clock).contains(&"Clock".to_string()));
        assert!(labels("keyb", MAC).contains(&"Keyboard settings".to_string()));
    }

    #[test]
    fn only_the_ones_that_end_the_session_ask_first() {
        assert!(
            Power::Restart.confirms() && Power::ShutDown.confirms() && Power::LogOut.confirms()
        );
        assert!(!Power::Lock.confirms() && !Power::Sleep.confirms());
        assert_eq!(Power::Restart.question("swift"), "Restart swift?");
        assert_eq!(Power::LogOut.question("swift"), "Log out?");
    }

    #[test]
    fn timers_parse_a_duration_and_a_name() {
        assert_eq!(timer_spec("t 5m"), Some(("5m".into(), None)));
        assert_eq!(
            timer_spec("t 5m tea  time"),
            Some(("5m".into(), Some("tea  time".into())))
        );
        assert_eq!(timer_spec("timer 1h30m"), Some(("1h30m".into(), None)));
        assert_eq!(timer_spec("t"), None);
        assert_eq!(timer_spec("t tea"), None);
        assert_eq!(timer_spec("sleep 5"), None);
    }

    #[test]
    fn pane_urls_use_the_settings_scheme() {
        assert_eq!(
            pane_url("com.apple.Sound-Settings.extension"),
            "x-apple.systempreferences:com.apple.Sound-Settings.extension"
        );
    }

    /// Reads the System Settings extensions on this Mac, so a renamed pane
    /// fails here rather than opening the wrong page.
    #[cfg(target_os = "macos")]
    #[test]
    fn every_pane_exists_on_this_macos() {
        let dir = std::path::Path::new("/System/Library/ExtensionKit/Extensions");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let ids: std::collections::HashSet<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| plist::Value::from_file(e.path().join("Contents/Info.plist")).ok())
            .filter_map(|v| {
                v.as_dictionary()?
                    .get("CFBundleIdentifier")?
                    .as_string()
                    .map(String::from)
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        for (label, id, _) in PANES {
            assert!(
                ids.contains(id),
                "{label}: {id} is not a System Settings pane here"
            );
        }
    }
}
