mod app;
mod backend;
mod capture;
mod cover;
mod eq;
mod history;
mod identify;
mod model;
mod motion;
mod rainbow;
mod response;
mod song;
mod spectrum;
mod ui;

use app::App;
use model::Snapshot;
use std::{cell::RefCell, rc::Rc, time::Duration};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("eq-seed") {
        return eq_seed().await;
    }
    let args = telmo_kit::cli::args();
    if args.status {
        return status(args.mock).await;
    }

    let cached: Option<Snapshot> = telmo_kit::cache::load("sound");
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    let last = Rc::new(RefCell::new(cached.clone()));
    let mut app = App::new(cached, cmd_tx, last.clone(), event_tx.clone(), args.mock);
    app.set_picker(telmo_kit::images::picker());
    backend::spawn(args.mock, cmd_rx, event_tx);

    let result = telmo_kit::run(app, event_rx).await;
    if let Some(snapshot) = last.borrow().as_ref() {
        telmo_kit::cache::save("sound", snapshot);
    }
    if let Err(e) = result {
        eprintln!("telmo-sound: the terminal failed: {e}");
        std::process::exit(1);
    }
}

async fn status(mock: bool) {
    match first_snapshot(mock).await {
        Some(snapshot) => match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => println!("{json}"),
            Err(e) => fail(&format!("could not encode the snapshot: {e}")),
        },
        None => fail("the audio system did not answer within 5 seconds. Is it running?"),
    }
}

/// Gives every output that has no EQ settings yet its default preset, then
/// exits. Telmo.app runs this when it meets an output the popup hasn't seen.
async fn eq_seed() {
    let Some(snapshot) = first_snapshot(false).await else {
        fail("the audio system did not answer within 5 seconds. Is it running?");
    };
    if let Err(e) = eq::Config::update(|config| {
        config.seed(&snapshot.outputs);
    }) {
        fail(&format!("could not save the EQ settings: {e}"));
    }
}

async fn first_snapshot(mock: bool) -> Option<Snapshot> {
    let (_cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, mut event_rx) = unbounded_channel();
    backend::spawn(mock, cmd_rx, event_tx);
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = event_rx.recv().await {
            if let backend::Event::Snapshot(snapshot) = event {
                return Some(snapshot);
            }
        }
        None
    })
    .await;
    first.ok().flatten()
}

fn fail(message: &str) -> ! {
    eprintln!("telmo-sound: {message}");
    std::process::exit(1);
}
