mod actions;
mod app;
mod apps;
mod canvas;
mod effects;
mod prefs;
mod rebuild;
mod tracker;
mod ui;

use serde_json::json;
use std::process::ExitCode;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("rebuild-run") {
        return rebuild::main(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("apps") {
        return print_apps();
    }
    if args.first().map(String::as_str) == Some("action") {
        return run_action(args.get(1).map(String::as_str));
    }
    let cli = telmo_kit::cli::args();
    let resolved = prefs::load();
    if cli.status {
        return status(&resolved);
    }
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    let apps = apps::spawn(cli.mock, event_tx.clone());
    actions::spawn(cli.mock, cmd_rx, event_tx);

    let mut app = app::App::new(cmd_tx, resolved, app::host_name()).with_apps(apps);
    if cli.mock {
        app.use_mock_rebuild();
    } else if let Some(view) = take_intent() {
        app.start_intent(&view);
    }
    match telmo_kit::run(app, event_rx).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("telmo-system: Could not use the terminal: {e}");
            ExitCode::FAILURE
        }
    }
}

fn status(resolved: &prefs::Resolved) -> ExitCode {
    let status = json!({
        "effect": resolved.prefs.effect,
        "logo": resolved.prefs.logo,
        "effects": resolved.cycle,
    });
    match serde_json::to_string_pretty(&status) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("telmo-system: Could not print the status: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Hidden: prints the real app list the Force Quit view would show.
fn print_apps() -> ExitCode {
    let printed = apps::sample_once()
        .and_then(|rows| serde_json::to_string_pretty(&rows).map_err(|e| e.to_string()));
    match printed {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("telmo-system: Could not list the apps: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `telmo-system action lock|sleep|restart|shutdown|logout`: no UI, no
/// question asked. The launcher runs this after its own confirmation.
fn run_action(word: Option<&str>) -> ExitCode {
    let Some(cmd) = word.and_then(actions::Cmd::parse) else {
        eprintln!("usage: telmo-system action lock|sleep|restart|shutdown|logout");
        return ExitCode::from(2);
    };
    match actions::run_once(cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("telmo-system: {message}");
            ExitCode::FAILURE
        }
    }
}

/// What the launcher left for this popup to start (`rebuild`, `apps`): read
/// once, then removed, and ignored when it is stale.
fn take_intent() -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Intent {
        view: String,
        at: u64,
    }
    let intent: Intent = telmo_kit::state::load("system-intent")?;
    let _ = std::fs::remove_file(telmo_kit::state::dir()?.join("system-intent.json"));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    (now.saturating_sub(intent.at) <= 15).then_some(intent.view)
}
