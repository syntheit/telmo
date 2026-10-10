mod app;
mod backend;
mod model;
mod ui;

use backend::Event;
use std::{process::ExitCode, time::Duration};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = telmo_kit::cli::args();
    let (event_tx, event_rx) = unbounded_channel();
    let backend = backend::spawn(args.mock, event_tx);

    let code = if args.status {
        status(event_rx).await
    } else {
        match telmo_kit::run(app::App::new(), event_rx).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&format!("Could not use the terminal: {e}")),
        }
    };
    // The event receiver is gone by now; let the backend stop the trackpad.
    let _ = tokio::time::timeout(Duration::from_secs(2), backend).await;
    code
}

/// Print the first snapshot the backend sends as JSON.
async fn status(mut events: UnboundedReceiver<Event>) -> ExitCode {
    let Some(snapshot) =
        telmo_kit::cli::first_snapshot(&mut events, |Event::Snapshot(snapshot)| Some(snapshot))
            .await
    else {
        return fail("The trackpad did not answer within 5 seconds. Try again.");
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
    eprintln!("telmo-scale: {message}");
    ExitCode::FAILURE
}
