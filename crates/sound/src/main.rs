mod app;
mod backend;
mod capture;
mod cover;
mod history;
mod identify;
mod model;
mod motion;
mod rainbow;
mod song;
mod spectrum;
mod ui;

use app::App;
use model::Snapshot;
use std::{cell::RefCell, rc::Rc, time::Duration};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = telmo_kit::cli::args();
    if args.status {
        return status(args.mock).await;
    }

    let cached: Option<Snapshot> = telmo_kit::cache::load("sound");
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (event_tx, event_rx) = unbounded_channel();
    let last = Rc::new(RefCell::new(cached.clone()));
    let mut app = App::new(cached, cmd_tx, last.clone(), event_tx.clone(), args.mock);
    app.set_picker(image_picker());
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

/// Asks the terminal which image protocol it speaks and how big a cell is.
/// Without an answer the covers are drawn in half-blocks.
fn image_picker() -> ratatui_image::picker::Picker {
    ratatui_image::picker::Picker::from_query_stdio()
        .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks())
}

async fn status(mock: bool) {
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
    match first {
        Ok(Some(snapshot)) => match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => println!("{json}"),
            Err(e) => fail(&format!("could not encode the snapshot: {e}")),
        },
        _ => fail("the audio system did not answer within 5 seconds. Is it running?"),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("telmo-sound: {message}");
    std::process::exit(1);
}
