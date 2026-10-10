mod app;
mod backend;
mod commands;
mod config;
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod desktop;
mod history;
mod math;
mod mode;
mod model;
mod rank;
mod ui;
mod web;

use backend::{CACHE, Source};
use model::AppEntry;
use serde_json::json;
use std::{process::ExitCode, sync::Arc};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("open") => return open(&args[1..]),
        Some("apps") => return print_apps(),
        _ => {}
    }
    let cli = telmo_kit::cli::args();
    let config = match config::load() {
        Ok(config) => config,
        Err(message) => return fail(&message),
    };
    if cli.status {
        return status(&config);
    }

    let source: Arc<dyn Source> = if cli.mock {
        Arc::new(backend::mock::Mock)
    } else {
        Arc::new(backend::Real)
    };
    // The cached list paints the first frame; the scan behind it corrects it.
    let cached: Vec<AppEntry> = if cli.mock {
        Vec::new()
    } else {
        telmo_kit::cache::load(CACHE).unwrap_or_default()
    };
    let history = if cli.mock {
        history::History::default()
    } else {
        history::History::load()
    };
    let env = commands::Env {
        macos: cfg!(target_os = "macos"),
        clock: source.clock_installed(),
    };
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = unbounded_channel();
    let mut app = app::App::new(cmd_tx, config, history, cached.clone(), env, host_name());
    app.set_picker(telmo_kit::images::picker_without_query());
    backend::spawn(source, cmd_rx, event_tx, cached);
    match telmo_kit::run(app, event_rx).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&format!("Could not use the terminal: {e}")),
    }
}

/// `telmo-launcher open <app name>`: opens the app, or brings it forward,
/// without any UI. For global hotkeys.
fn open(words: &[String]) -> ExitCode {
    let name = words.join(" ");
    if name.trim().is_empty() {
        return fail("usage: telmo-launcher open <app name>");
    }
    let source = backend::Real;
    let mut apps: Vec<AppEntry> = telmo_kit::cache::load(CACHE).unwrap_or_default();
    let mut found = model::find_named(&apps, &name).cloned();
    if found.is_none() {
        // Not in the cache (new app, or no cache yet): look on disk.
        match source.scan() {
            Ok(scanned) => apps = scanned,
            Err(message) => return fail(&message),
        }
        found = model::find_named(&apps, &name).cloned();
    }
    let Some(app) = found else {
        return fail(&format!("no app called '{name}' is installed"));
    };
    match source.open(&app) {
        Ok(_) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = history::save_open("", &app.id, now);
            ExitCode::SUCCESS
        }
        Err(message) => fail(&message),
    }
}

/// Hidden: prints the apps a fresh scan finds.
fn print_apps() -> ExitCode {
    match backend::Real.scan() {
        Ok(apps) => match serde_json::to_string_pretty(&apps) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&format!("Could not print the apps: {e}")),
        },
        Err(message) => fail(&message),
    }
}

fn status(config: &config::Config) -> ExitCode {
    let cached: Vec<AppEntry> = telmo_kit::cache::load(CACHE).unwrap_or_default();
    let engines: Vec<_> = web::engines(config).into_iter().map(|e| e.key).collect();
    let status = json!({
        "cachedApps": cached.len(),
        "keys": config.keys,
        "hotkeyLabel": config.hotkey_label,
        "engines": engines,
        "clock": backend::Real.clock_installed(),
    });
    match serde_json::to_string_pretty(&status) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("Could not print the status: {e}")),
    }
}

fn host_name() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates
    // unless the name was truncated, and we cut at the first NUL or the end.
    let ok = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0;
    if !ok {
        return "this computer".into();
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
    name.strip_suffix(".local").unwrap_or(&name).to_string()
}

fn fail(message: &str) -> ExitCode {
    eprintln!("telmo-launcher: {message}");
    ExitCode::FAILURE
}
