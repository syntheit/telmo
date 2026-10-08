mod actions;
mod app;
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
    if std::env::args().nth(1).as_deref() == Some("rebuild-run") {
        return rebuild::main(prefs::load().rebuild, app::host_name());
    }
    let args = telmo_kit::cli::args();
    let resolved = prefs::load();
    if args.status {
        return status(&resolved);
    }
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    actions::spawn(args.mock, cmd_rx, event_tx);

    let mut app = app::App::new(cmd_tx, resolved, app::host_name());
    if args.mock {
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
