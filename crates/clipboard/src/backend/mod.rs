//! Everything slow or system-facing runs here, off the UI thread.
//!
//! One thread owns the commands and watches the index for changes made by the
//! watcher; loading a picture happens on a thread of its own so a big image
//! never holds up a pin or a delete.

mod copy;
pub mod mock;
mod real;

use crate::model::Snapshot;
use image::DynamicImage;
use std::{
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

pub use real::Real;

/// How often the index is checked for outside changes.
const POLL: Duration = Duration::from_millis(500);
/// Pictures are shrunk to this many pixels on the long side before display.
const MAX_PIXELS: u32 = 1600;

/// UI -> backend.
#[derive(Debug, Clone)]
pub enum Cmd {
    Pin {
        id: String,
        pinned: bool,
    },
    Delete(String),
    Clear,
    /// Put an item (or, for `h`, another notation of it) back on the clipboard.
    Copy {
        id: String,
        text: Option<String>,
        close: bool,
    },
    /// The whole text of an item longer than its preview.
    LoadText(String),
    LoadImage(String),
}

/// Backend -> UI.
pub enum Event {
    Snapshot(Snapshot),
    Text {
        id: String,
        text: String,
    },
    /// `None`: the picture couldn't be read.
    Image {
        id: String,
        image: Option<DynamicImage>,
    },
    /// `Ok(None)`: done; `Ok(Some(message))`: done, tell the user.
    Copied {
        close: bool,
        result: Result<Option<String>, String>,
    },
    Failed(String),
}

pub type Tx = UnboundedSender<Event>;

/// Where the history lives: the real store, or the mock's memory.
pub trait Source: Send + Sync {
    fn snapshot(&self) -> Result<Snapshot, String>;
    /// Changes whenever the history does; `None` if it can't tell.
    fn version(&self) -> Option<std::time::SystemTime>;
    fn pin(&self, id: &str, pinned: bool) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
    fn text(&self, id: &str) -> Result<String, String>;
    fn image(&self, id: &str) -> Result<DynamicImage, String>;
    /// `Ok(Some(message))` when nothing was really copied (the mock).
    fn copy(&self, id: &str, text: Option<&str>) -> Result<Option<String>, String>;
}

pub fn spawn(source: Arc<dyn Source>, cmds: Receiver<Cmd>, events: Tx) {
    std::thread::spawn(move || run(source, cmds, events));
}

fn run(source: Arc<dyn Source>, cmds: Receiver<Cmd>, events: Tx) {
    let mut seen = None;
    send_snapshot(&*source, &events, &mut seen);
    loop {
        match cmds.recv_timeout(POLL) {
            Ok(cmd) => {
                handle(&source, cmd, &events);
                send_snapshot(&*source, &events, &mut seen);
            }
            Err(RecvTimeoutError::Timeout) => {
                if source.version() != seen {
                    send_snapshot(&*source, &events, &mut seen);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn send_snapshot(source: &dyn Source, events: &Tx, seen: &mut Option<std::time::SystemTime>) {
    *seen = source.version();
    let event = match source.snapshot() {
        Ok(snapshot) => Event::Snapshot(snapshot),
        Err(message) => Event::Failed(message),
    };
    let _ = events.send(event);
}

fn handle(source: &Arc<dyn Source>, cmd: Cmd, events: &Tx) {
    let failed = |result: Result<(), String>| {
        if let Err(message) = result {
            let _ = events.send(Event::Failed(message));
        }
    };
    match cmd {
        Cmd::Pin { id, pinned } => failed(source.pin(&id, pinned)),
        Cmd::Delete(id) => failed(source.delete(&id)),
        Cmd::Clear => failed(source.clear()),
        Cmd::LoadText(id) => match source.text(&id) {
            Ok(text) => {
                let _ = events.send(Event::Text { id, text });
            }
            Err(message) => failed(Err(message)),
        },
        Cmd::LoadImage(id) => load_image(source.clone(), id, events.clone()),
        Cmd::Copy { id, text, close } => {
            let result = source.copy(&id, text.as_deref());
            let _ = events.send(Event::Copied { close, result });
        }
    }
}

fn load_image(source: Arc<dyn Source>, id: String, events: Tx) {
    std::thread::spawn(move || {
        let image = source.image(&id).ok().map(|image| {
            if image.width().max(image.height()) > MAX_PIXELS {
                image.thumbnail(MAX_PIXELS, MAX_PIXELS)
            } else {
                image
            }
        });
        let _ = events.send(Event::Image { id, image });
    });
}
