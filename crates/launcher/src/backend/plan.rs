//! What running a command means in terms of programs.

use crate::commands::Action;

/// A program to run to completion, found next to us or on PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub program: &'static str,
    pub args: Vec<String>,
    /// Said when the program isn't there.
    pub missing: &'static str,
}

pub fn plan_for(action: &Action) -> Plan {
    let telmo = |args: Vec<String>| Plan {
        program: "telmo",
        args,
        missing: "telmo isn't on PATH, so its popups can't open.",
    };
    match action {
        Action::Popup(name) => telmo(vec!["popup".into(), name.to_string()]),
        Action::SystemView(_) => telmo(vec!["popup".into(), "system".into()]),
        Action::Power(power) => Plan {
            program: "telmo-system",
            args: vec!["action".into(), power.arg().into()],
            missing: "telmo-system isn't installed.",
        },
        Action::Settings(url) => Plan {
            program: if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            },
            args: vec![url.clone()],
            missing: "Couldn't open System Settings.",
        },
        Action::Timer { duration, name } => {
            let mut args = vec!["timer".to_string(), "start".into(), duration.clone()];
            if let Some(name) = name {
                args.push("--name".into());
                args.push(name.clone());
            }
            Plan {
                program: "telmo-clock",
                args,
                missing: "Clock isn't installed.",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Power;

    #[test]
    fn actions_become_programs() {
        assert_eq!(plan_for(&Action::Popup("net")).args, ["popup", "net"]);
        let sleep = plan_for(&Action::Power(Power::Sleep));
        assert_eq!(
            (sleep.program, sleep.args),
            ("telmo-system", vec!["action".to_string(), "sleep".into()])
        );
        let system = plan_for(&Action::SystemView("rebuild"));
        assert_eq!(system.args, ["popup", "system"]);
    }

    #[test]
    fn a_timer_carries_its_name_only_when_it_has_one() {
        let plain = plan_for(&Action::Timer {
            duration: "5m".into(),
            name: None,
        });
        assert_eq!(plain.args, ["timer", "start", "5m"]);
        let named = plan_for(&Action::Timer {
            duration: "90s".into(),
            name: Some("tea time".into()),
        });
        assert_eq!(named.program, "telmo-clock");
        assert_eq!(named.args, ["timer", "start", "90s", "--name", "tea time"]);
        assert_eq!(named.missing, "Clock isn't installed.");
    }
}
