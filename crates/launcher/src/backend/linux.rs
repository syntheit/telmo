//! Linux: `.desktop` files for the apps, Hyprland for windows, `wl-copy` to copy.

use super::xdg::{data_dirs, icon_path, scan_dirs};
use super::{Source, hypr, sys};
use crate::commands::Action;
use crate::desktop::{Env, exec_args};
use crate::model::AppEntry;
use image::DynamicImage;
use std::{
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
};

pub struct Real;

fn terminal() -> String {
    ["TELMO_TERMINAL", "TERMINAL"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|t| !t.is_empty()))
        .unwrap_or_else(|| "ghostty".to_string())
}

impl Source for Real {
    fn scan(&self) -> Result<Vec<AppEntry>, String> {
        Ok(scan_dirs(&data_dirs(), &Env::from_environment()))
    }

    fn running(&self, apps: &[AppEntry]) -> HashSet<String> {
        let clients = hypr::clients();
        apps.iter()
            .filter(|app| hypr::window_of(app, &clients).is_some())
            .map(|app| app.id.clone())
            .collect()
    }

    fn icon(&self, app: &AppEntry) -> Option<DynamicImage> {
        let path = icon_path(app.icon.as_deref()?, &data_dirs())?;
        image::open(path).ok()
    }

    fn open(&self, app: &AppEntry) -> Result<Option<String>, String> {
        if let Some(client) = hypr::window_of(app, &hypr::clients()) {
            return hypr::focus(&client.address).map(|()| None);
        }
        let exec = app
            .exec
            .as_deref()
            .ok_or("This app has no command to run.")?;
        let mut words = exec_args(exec, &app.name, app.icon.as_deref(), &app.path);
        if app.terminal {
            let mut wrapped = vec![terminal(), "-e".to_string()];
            wrapped.append(&mut words);
            words = wrapped;
        }
        let (program, args) = words
            .split_first()
            .ok_or("This app has no command to run.")?;
        sys::spawn_detached(program, args).map(|()| None)
    }

    fn copy(&self, text: &str) -> Result<Option<String>, String> {
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't run wl-copy: {e}. Is wl-clipboard installed?"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("wl-copy stopped listening: {e}."))?;
        }
        // wl-copy hands the text to a background process and exits.
        let status = child.wait().map_err(|e| format!("wl-copy failed: {e}."))?;
        if status.success() {
            Ok(None)
        } else {
            Err("wl-copy failed. Is a Wayland session running?".into())
        }
    }

    fn open_url(&self, url: &str) -> Result<Option<String>, String> {
        telmo_kit::os::open(url).map(|()| None)
    }

    fn run(&self, action: &Action) -> Result<Option<String>, String> {
        sys::run_action(action)
    }

    fn clock_installed(&self) -> bool {
        sys::clock_installed()
    }
}
