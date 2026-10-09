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
        return rebuild::main(&args[1..], &app::host_name());
    }
    if args.first().map(String::as_str) == Some("apps") {
        return print_apps();
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
