mod app;
mod backend;
mod color;
mod config;
mod content;
mod detect;
mod ingest;
mod model;
mod os;
mod search;
mod store;
mod timefmt;
mod ui;
mod wayland;

use serde_json::json;
use std::{process::ExitCode, sync::Arc};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ingest") => return ingest::run(&args[1..]),
        Some("ingest-wayland") => return ingest::run_wayland(),
        _ => {}
    }
    let cli = telmo_kit::cli::args();
    let source: Arc<dyn backend::Source> = if cli.mock {
        Arc::new(backend::mock::Mock::new(ingest::now()))
    } else {
        match ingest::open_store() {
            Ok(store) => Arc::new(backend::Real(store)),
            Err(message) => return fail(&message),
        }
    };
    if cli.status {
        return status(&*source);
    }

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = unbounded_channel();
    backend::spawn(source, cmd_rx, event_tx);
    let mut app = app::App::new(cmd_tx);
    app.set_picker(telmo_kit::images::picker());
    match telmo_kit::run(app, event_rx).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&format!("Could not use the terminal: {e}")),
    }
}

/// Asks the terminal which image protocol it speaks and how big a cell is.
/// Without an answer pictures are drawn in half-blocks. `TELMO_IMAGE_PROTOCOL`
/// overrides the protocol for hosts that know better than their own answer
/// (Telmo.app's SwiftTerm claims kitty graphics but never draws ratatui-image's
/// placements, which carry no size).
fn status(source: &dyn backend::Source) -> ExitCode {
    let snapshot = match source.snapshot() {
        Ok(snapshot) => snapshot,
        Err(message) => return fail(&message),
    };
    let dir = config::data_dir().map(|d| d.display().to_string());
    let status = json!({
        "items": snapshot.entries.len(),
        "pinned": snapshot.entries.iter().filter(|e| e.pinned).count(),
        "dir": dir,
        "maxItems": snapshot.max_items,
        "maxDays": snapshot.max_days,
    });
    match serde_json::to_string_pretty(&status) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("Could not print the status: {e}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("telmo-clipboard: {message}");
    ExitCode::FAILURE
}
