//! Hyprland backend. Reads once when the app opens and again after every
//! action; nothing polls. Brightness comes from `brightnessctl` (built-in
//! panel) and `ddcutil` (external monitors), Night Shift from `hyprsunset`,
//! sizes from `hyprctl`. A missing tool only turns its rows read-only.

use super::{Cmd, Event, Rx, SCALE_NOTE, Tx};
use crate::model::{Display, Info, Kind, Mode, Night, Snapshot};
use serde_json::Value;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
/// Night Shift warmth maps onto this range of colour temperatures.
const NEUTRAL_KELVIN: f32 = 6500.0;
const WARMEST_KELVIN: f32 = 2500.0;

pub fn spawn(mut cmds: Rx, events: Tx) {
    std::thread::spawn(move || {
        let mut system = System::default();
        publish(&mut system, &events);
        while let Some(cmd) = cmds.blocking_recv() {
            if let Err(text) = system.execute(cmd) {
                let _ = events.send(Event::Failed(text));
            }
            publish(&mut system, &events);
        }
    });
}

fn publish(system: &mut System, events: &Tx) {
    let snapshot = match system.snapshot() {
        Ok(snapshot) => snapshot,
        Err(text) => {
            let _ = events.send(Event::Failed(text));
            Snapshot::default()
        }
    };
    let _ = events.send(Event::Snapshot(snapshot));
}

#[derive(Default)]
struct System {
    /// ddcutil display numbers by connector name, found on first use.
    ddc_numbers: Option<HashMap<String, u32>>,
    /// Brightness and maximum read or set over DDC; reading is slow.
    ddc_levels: HashMap<String, (f32, u32)>,
}

impl System {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        let monitors = hyprctl(&["monitors", "-j"])?;
        let monitors: Vec<Value> = serde_json::from_str(&monitors).map_err(|_| {
            "Hyprland gave an unreadable monitor list. Is it up to date?".to_string()
        })?;
        let displays = monitors.iter().map(|m| self.display(m)).collect();
        Ok(Snapshot {
            displays,
            night: night(),
        })
    }

    fn display(&mut self, monitor: &Value) -> Display {
        let connector = text(monitor, "name");
        let builtin = is_builtin(&connector);
        let width = monitor["width"].as_u64().unwrap_or(0);
        let height = monitor["height"].as_u64().unwrap_or(0);
        let scale = monitor["scale"].as_f64().unwrap_or(1.0) as f32;
        let refresh = monitor["refreshRate"].as_f64().map(|hz| hz as f32);
        Display {
            id: connector.clone(),
            name: monitor_name(monitor, &connector),
            kind: if builtin {
                Kind::Builtin
            } else {
                Kind::External
            },
            brightness: self.brightness(&connector, builtin),
            info: Info {
                resolution: (width > 0).then(|| format!("{width} × {height}")),
                refresh,
                scale: Some(scale),
                connection: Some(connector),
            },
            modes: scale_modes(width, height, scale),
            modes_note: Some(SCALE_NOTE.into()),
        }
    }

    fn brightness(&mut self, connector: &str, builtin: bool) -> Option<f32> {
        if builtin {
            return backlight().map(|(level, _)| level);
        }
        if let Some((level, _)) = self.ddc_levels.get(connector) {
            return Some(*level);
        }
        let number = self.ddc_number(connector)?;
        let (level, max) = ddc_get(number)?;
        self.ddc_levels.insert(connector.into(), (level, max));
        Some(level)
    }

    fn ddc_number(&mut self, connector: &str) -> Option<u32> {
        let numbers = self.ddc_numbers.get_or_insert_with(ddc_detect);
        numbers.get(connector).copied()
    }

    fn execute(&mut self, cmd: Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Brightness { display, value } => self.set_brightness(&display, value),
            Cmd::Night { on, warmth } => set_night(on, warmth),
            Cmd::Size { display, mode } => set_scale(&display, &mode),
        }
    }

    fn set_brightness(&mut self, connector: &str, value: f32) -> Result<(), String> {
        if is_builtin(connector) {
            let percent = format!("{}%", (value * 100.0).round() as u32);
            return run("brightnessctl", &["-c", "backlight", "set", &percent]).map(drop);
        }
        let number = self
            .ddc_number(connector)
            .ok_or("This monitor does not take brightness commands. Use its own buttons.")?;
        let max = self.ddc_levels.get(connector).map_or(100, |(_, max)| *max);
        let raw = (value * max as f32).round() as u32;
        run(
            "ddcutil",
            &[
                "setvcp",
                "10",
                &raw.to_string(),
                "--display",
                &number.to_string(),
            ],
        )?;
        self.ddc_levels.insert(connector.into(), (value, max));
        Ok(())
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn is_builtin(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|p| connector.starts_with(p))
}

fn monitor_name(monitor: &Value, connector: &str) -> String {
    let model = text(monitor, "model");
    let make = text(monitor, "make");
    match (make.is_empty(), model.is_empty()) {
        (_, false) if model.starts_with(&make) => model,
        (false, false) => format!("{make} {model}"),
        (true, false) => model,
        _ => connector.to_string(),
    }
}

// ---- Hyprland ----

/// Runs `hyprctl`, finding the running instance when the environment (an ssh
/// session, say) does not say which one.
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("hyprctl");
    command.args(args);
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        let uid = std::fs::metadata("/proc/self").map_or(1000, |m| m.uid());
        format!("/run/user/{uid}")
    });
    command.env("XDG_RUNTIME_DIR", &runtime);
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
        && let Some(signature) = newest_instance(&runtime)
    {
        command.env("HYPRLAND_INSTANCE_SIGNATURE", signature);
    }
    let output = command
        .output()
        .map_err(|_| "hyprctl is not installed, so Hyprland can't be read.".to_string())?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.contains("HYPRLAND_INSTANCE_SIGNATURE not set") {
        return Err("Hyprland is not running, so there is nothing to show.".into());
    }
    if output.status.success() {
        Ok(out)
    } else {
        Err(format!(
            "hyprctl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn newest_instance(runtime: &str) -> Option<String> {
    let dir = std::fs::read_dir(format!("{runtime}/hypr")).ok()?;
    let newest = dir
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.file_name())))
        .max()?;
    newest.1.into_string().ok()
}

/// The scale choices, each with what fits on screen. A scale outside the list
/// (set in the config) is added so the current one is always shown.
fn scale_modes(width: u64, height: u64, current: f32) -> Vec<Mode> {
    let mut scales = SCALES.to_vec();
    if !scales.iter().any(|s| same_scale(*s, current)) {
        scales.push(current);
        scales.sort_by(f32::total_cmp);
    }
    scales
        .into_iter()
        .map(|scale| Mode {
            id: scale.to_string(),
            label: format!("{scale}x"),
            note: format!(
                "{} × {}",
                (width as f32 / scale).round(),
                (height as f32 / scale).round()
            ),
            current: same_scale(scale, current),
        })
        .collect()
}

fn same_scale(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// `hyprctl keyword` only changes the running session; the monitor keeps its
/// current mode and position so only the scale moves.
fn set_scale(connector: &str, scale: &str) -> Result<(), String> {
    let monitors = hyprctl(&["monitors", "-j"])?;
    let monitors: Vec<Value> = serde_json::from_str(&monitors)
        .map_err(|_| "Hyprland gave an unreadable monitor list.".to_string())?;
    let monitor = monitors
        .iter()
        .find(|m| m["name"] == connector)
        .ok_or("That monitor is gone. Reopen the app to refresh the list.")?;
    let rule = format!(
        "{connector},{}x{}@{:.2},{}x{},{scale}",
        monitor["width"],
        monitor["height"],
        monitor["refreshRate"].as_f64().unwrap_or(60.0),
        monitor["x"],
        monitor["y"],
    );
    let reply = hyprctl(&["keyword", "monitor", &rule])?;
    if reply == "ok" {
        Ok(())
    } else {
        Err(format!("Hyprland refused the scale: {reply}"))
    }
}

// ---- Night Shift ----

fn night() -> Night {
    let kelvin = hyprctl(&["hyprsunset", "temperature"])
        .ok()
        .and_then(|out| out.parse::<f32>().ok());
    match kelvin {
        Some(k) => Night {
            available: true,
            on: k < NEUTRAL_KELVIN - 1.0,
            warmth: ((NEUTRAL_KELVIN - k) / (NEUTRAL_KELVIN - WARMEST_KELVIN)).clamp(0.0, 1.0),
        },
        None => Night::default(),
    }
}

fn set_night(on: bool, warmth: f32) -> Result<(), String> {
    let reply = if on {
        let kelvin = (NEUTRAL_KELVIN - warmth * (NEUTRAL_KELVIN - WARMEST_KELVIN)).round();
        hyprctl(&["hyprsunset", "temperature", &kelvin.to_string()])?
    } else {
        hyprctl(&["hyprsunset", "identity"])?
    };
    if reply.is_empty() || reply == "ok" {
        Ok(())
    } else {
        Err(format!("hyprsunset said: {reply}. Is it running?"))
    }
}

// ---- Brightness tools ----

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|_| format!("{program} is not installed, so brightness is read-only."))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(format!("{program} failed: {}", error.trim()))
    }
}

/// (level 0.0-1.0, maximum) of the built-in backlight, if brightnessctl sees one.
fn backlight() -> Option<(f32, u32)> {
    let out = run("brightnessctl", &["-m", "-c", "backlight", "info"]).ok()?;
    // name,class,current,percent,max
    let fields: Vec<&str> = out.lines().next()?.split(',').collect();
    let max: u32 = fields.get(4)?.parse().ok()?;
    let current: f32 = fields.get(2)?.parse().ok()?;
    Some(((current / max as f32).clamp(0.0, 1.0), max))
}

/// ddcutil display numbers by connector ("DP-1"), from `ddcutil detect`.
fn ddc_detect() -> HashMap<String, u32> {
    let mut numbers = HashMap::new();
    let Ok(out) = run("ddcutil", &["detect", "--brief"]) else {
        return numbers;
    };
    let mut current = None;
    for line in out.lines() {
        let line = line.trim();
        if let Some(n) = line.strip_prefix("Display ") {
            current = n.parse().ok();
        } else if let (Some(n), Some(connector)) = (current, line.strip_prefix("DRM connector:")) {
            // "card1-DP-1": the card prefix is not part of Hyprland's name.
            let name = connector
                .trim()
                .split_once('-')
                .map_or("", |(_, name)| name);
            numbers.insert(name.to_string(), n);
        }
    }
    numbers
}

fn ddc_get(number: u32) -> Option<(f32, u32)> {
    let out = run(
        "ddcutil",
        &["getvcp", "10", "--display", &number.to_string(), "--brief"],
    )
    .ok()?;
    // VCP 10 C current max
    let fields: Vec<&str> = out.split_whitespace().collect();
    let current: f32 = fields.get(3)?.parse().ok()?;
    let max: u32 = fields.get(4)?.parse().ok()?;
    (max > 0).then(|| ((current / max as f32).clamp(0.0, 1.0), max))
}
