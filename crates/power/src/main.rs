mod app;
mod backend;
mod model;
mod ui;

use backend::Event;
use model::Snapshot;
use std::{process::ExitCode, time::Duration};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = telmo_kit::cli::args();
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    let backend = backend::spawn(args.mock, cmd_rx, event_tx);

    let code = if args.status {
        let code = status(event_rx).await;
        drop(cmd_tx);
        code
    } else {
        let cached = telmo_kit::cache::load::<Snapshot>("power");
        let app = app::App::new(cmd_tx, cached).save_on_exit();
        match telmo_kit::run(app, event_rx).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&format!("Could not use the terminal: {e}")),
        }
    };
    if let Some(backend) = backend {
        let _ = tokio::time::timeout(Duration::from_secs(2), backend).await;
    }
    code
}

/// Print the first snapshot the backend sends as JSON.
async fn status(mut events: UnboundedReceiver<Event>) -> ExitCode {
    let Some(snapshot) = telmo_kit::cli::first_snapshot(&mut events, |event| match event {
        Event::Snapshot(snapshot) => Some(snapshot),
        _ => None,
    })
    .await
    else {
        return fail("The power status did not arrive within 5 seconds. Is the system busy?");
    };
    match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("Could not print the snapshot: {e}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("telmo-power: {message}");
    ExitCode::FAILURE
}
