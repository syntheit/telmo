//! `--mock`: a realistic app list that is never launched, with drawn icons.

use super::Source;
use crate::commands::Action;
use crate::model::AppEntry;
use image::{DynamicImage, Rgba, RgbaImage};
use std::collections::HashSet;

pub struct Mock;

/// Name and bundle identifier.
const APPS: [(&str, &str); 26] = [
    ("1Password", "com.1password.1password"),
    ("Activity Monitor", "com.apple.ActivityMonitor"),
    ("Arc", "company.thebrowser.Browser"),
    ("Calculator", "com.apple.calculator"),
    ("Calendar", "com.apple.iCal"),
    ("Claude", "com.anthropic.claudefordesktop"),
    ("Cursor", "com.todesktop.cursor"),
    ("Ghostty", "com.mitchellh.ghostty"),
    ("Karabiner-Elements", "org.pqrs.Karabiner-Elements"),
    ("Mail", "com.apple.mail"),
    ("Messages", "com.apple.MobileSMS"),
    ("Notes", "com.apple.Notes"),
    ("Obsidian", "md.obsidian"),
    ("OrbStack", "dev.kdrag0n.MacVirt"),
    ("Preview", "com.apple.Preview"),
    ("Safari", "com.apple.Safari"),
    ("Spotify", "com.spotify.client"),
    ("System Settings", "com.apple.systempreferences"),
    ("Telegram", "ru.keepcoder.Telegram"),
    ("Terminal", "com.apple.Terminal"),
    ("TextEdit", "com.apple.TextEdit"),
    ("Thunderbird", "org.mozilla.thunderbird"),
    ("Visual Studio Code", "com.microsoft.VSCode"),
    ("VLC", "org.videolan.vlc"),
    ("Xcode", "com.apple.dt.Xcode"),
    ("Zen Browser", "app.zen-browser.zen"),
];

/// Running in the mock.
const RUNNING: [&str; 5] = ["Ghostty", "Zen Browser", "Spotify", "Telegram", "Obsidian"];

pub fn apps() -> Vec<AppEntry> {
    APPS.iter()
        .map(|(name, id)| {
            let mut app = AppEntry::new(id, name, &format!("/Applications/{name}.app"));
            if *name == "Visual Studio Code" {
                app.aliases.push("Code".into());
            }
            app
        })
        .collect()
}

pub fn running() -> HashSet<String> {
    apps()
        .into_iter()
        .filter(|a| RUNNING.contains(&a.name.as_str()))
        .map(|a| a.id)
        .collect()
}

/// A square in a color taken from the name, lighter on top: enough to see
/// where icons go (a flat one would draw as blank half blocks).
pub fn icon(name: &str) -> DynamicImage {
    let hash = name.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    let channel = |shift: u32| 70 + ((hash >> shift) & 0x7f) as u8;
    let top = Rgba([channel(0), channel(8), channel(16), 255]);
    let bottom = Rgba([top[0] / 2, top[1] / 2, top[2] / 2, 255]);
    DynamicImage::ImageRgba8(RgbaImage::from_fn(32, 32, |_, y| {
        if y < 16 { top } else { bottom }
    }))
}

impl Source for Mock {
    fn scan(&self) -> Result<Vec<AppEntry>, String> {
        Ok(apps())
    }

    fn running(&self, _apps: &[AppEntry]) -> HashSet<String> {
        running()
    }

    fn icon(&self, app: &AppEntry) -> Option<DynamicImage> {
        Some(icon(&app.name))
    }

    fn open(&self, app: &AppEntry) -> Result<Option<String>, String> {
        Ok(Some(format!("Would open {}.", app.name)))
    }

    fn copy(&self, text: &str) -> Result<Option<String>, String> {
        Ok(Some(format!("Would copy {text}.")))
    }

    fn open_url(&self, url: &str) -> Result<Option<String>, String> {
        Ok(Some(format!("Would open {url}.")))
    }

    fn run(&self, action: &Action) -> Result<Option<String>, String> {
        let what = match action {
            Action::Popup(name) => format!("open the {name} popup"),
            Action::Power(power) => power.verb().to_string(),
            Action::SystemView(view) => format!("open System ({view})"),
            Action::Settings(url) => format!("open {url}"),
            Action::Timer { duration, .. } => format!("start a {duration} timer"),
        };
        Ok(Some(format!("Would {what}.")))
    }

    fn clock_installed(&self) -> bool {
        true
    }

    fn is_mock(&self) -> bool {
        true
    }
}
