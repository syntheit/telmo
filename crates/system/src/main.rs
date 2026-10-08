mod actions;
mod app;
// The effects module's API is only partly used until the real effects land.
#[allow(dead_code)]
mod canvas;
#[allow(dead_code)]
mod effects;
mod prefs;
mod ui;

use serde_json::json;
use std::process::ExitCode;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = telmo_kit::cli::args();
    let resolved = prefs::load();
    if args.status {
        return status(&resolved);
    }
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    actions::spawn(args.mock, cmd_rx, event_tx);

    let app = app::App::new(cmd_tx, resolved, app::host_name());
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
