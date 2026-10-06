mod app;
mod backend;
mod model;
mod ui;

use backend::Event;
use model::Snapshot;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = telmo_kit::cli::args();
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, mut event_rx) = unbounded_channel();
    backend::spawn(args.mock, cmd_rx, event_tx.clone());

    if args.status {
        return print_status(&mut event_rx).await;
    }

    let cached = if args.mock {
        None
    } else {
        telmo_kit::cache::load::<Snapshot>("net")
    };
    let app = app::App::new(cached.unwrap_or_default(), cmd_tx, event_tx, args.mock);
    if let Err(e) = telmo_kit::run(app, event_rx).await {
        eprintln!("telmo-net: terminal error: {e}");
        std::process::exit(1);
    }
}

/// Print the first snapshot the backend sends as JSON.
async fn print_status(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) {
    let first = async {
        while let Some(event) = events.recv().await {
            if let Event::Snapshot(snapshot) = event {
                return Some(snapshot);
            }
        }
        None
    };
    match tokio::time::timeout(Duration::from_secs(5), first).await {
        Ok(Some(snapshot)) => match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => println!("{json}"),
            Err(e) => eprintln!("telmo-net: couldn't format the snapshot: {e}"),
        },
        _ => {
            eprintln!("telmo-net: the network backend didn't answer within 5 seconds.");
            std::process::exit(1);
        }
    }
}
