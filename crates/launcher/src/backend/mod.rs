//! Everything slow or system-facing runs here, off the UI thread: finding the
//! apps, asking which are running, loading icons, launching and copying.

pub mod mock;
mod plan;
mod sys;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod bundle;
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub mod hypr;
#[cfg(not(target_os = "macos"))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod xdg;

#[cfg(not(target_os = "macos"))]
pub use linux::Real;
#[cfg(target_os = "macos")]
pub use macos::Real;

use crate::commands::Action;
use crate::history;
use crate::model::AppEntry;
use image::DynamicImage;
use std::{
    collections::HashSet,
    sync::{Arc, mpsc::Receiver},
};
use tokio::sync::mpsc::UnboundedSender;

/// Icons are shrunk to this many pixels on a side once loaded.
const ICON_PIXELS: u32 = 64;
/// The cached app list; painted at once while a fresh scan runs.
pub const CACHE: &str = "launcher-apps";

/// UI -> backend.
#[derive(Debug, Clone)]
pub enum Cmd {
    /// Open or focus an app; `query` is what was typed to find it.
    Open {
        app: AppEntry,
        query: String,
    },
    Copy(String),
    OpenUrl(String),
    Run(Action),
    Icon(AppEntry),
}

/// Backend -> UI.
pub enum Event {
    Apps(Vec<AppEntry>),
    /// Ids of the apps that are running.
    Running(HashSet<String>),
    /// `None`: the app has no icon we can draw.
    Icon {
        id: String,
        image: Option<DynamicImage>,
    },
    /// Done. `Some(message)` when nothing really happened (the mock) and the
    /// popup should stay to say so.
    Done(Option<String>),
    Failed(String),
}

pub type Tx = UnboundedSender<Event>;

/// The real system, or the mock's memory.
pub trait Source: Send + Sync {
    fn scan(&self) -> Result<Vec<AppEntry>, String>;
    /// Ids of the running apps among `apps`.
    fn running(&self, apps: &[AppEntry]) -> HashSet<String>;
    fn icon(&self, app: &AppEntry) -> Option<DynamicImage>;
    /// Starts the app, or brings it forward when it runs already.
    fn open(&self, app: &AppEntry) -> Result<Option<String>, String>;
    fn copy(&self, text: &str) -> Result<Option<String>, String>;
    fn open_url(&self, url: &str) -> Result<Option<String>, String>;
    fn run(&self, action: &Action) -> Result<Option<String>, String>;
    /// Whether `telmo-clock` is installed.
    fn clock_installed(&self) -> bool;
    /// Mocks learn nothing and cache nothing.
    fn is_mock(&self) -> bool {
        false
    }
}

pub fn spawn(source: Arc<dyn Source>, cmds: Receiver<Cmd>, events: Tx, apps: Vec<AppEntry>) {
    std::thread::spawn(move || run(source, cmds, events, apps));
}

fn run(source: Arc<dyn Source>, cmds: Receiver<Cmd>, events: Tx, cached: Vec<AppEntry>) {
    // Running first: it is quick, and the dots then match the cached list.
    let _ = events.send(Event::Running(source.running(&cached)));
    match source.scan() {
        Ok(apps) => {
            if apps != cached {
                if !source.is_mock() {
                    telmo_kit::cache::save(CACHE, &apps);
                }
                let _ = events.send(Event::Running(source.running(&apps)));
                let _ = events.send(Event::Apps(apps));
            }
        }
        Err(message) => {
            let _ = events.send(Event::Failed(message));
        }
    }
    while let Ok(cmd) = cmds.recv() {
        handle(&source, cmd, &events);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn handle(source: &Arc<dyn Source>, cmd: Cmd, events: &Tx) {
    let finish = |result: Result<Option<String>, String>| {
        let _ = events.send(match result {
            Ok(note) => Event::Done(note),
            Err(message) => Event::Failed(message),
        });
    };
    match cmd {
        Cmd::Open { app, query } => {
            let result = source.open(&app);
            if result.is_ok() && !source.is_mock() {
                let _ = history::save_open(&query, &app.id, now());
            }
            finish(result);
        }
        Cmd::Copy(text) => finish(source.copy(&text)),
        Cmd::OpenUrl(url) => finish(source.open_url(&url)),
        Cmd::Run(action) => finish(source.run(&action)),
        Cmd::Icon(app) => load_icon(source.clone(), app, events.clone()),
    }
}

/// Icons load on a thread each: the host may take a moment to draw a cold one.
fn load_icon(source: Arc<dyn Source>, app: AppEntry, events: Tx) {
    std::thread::spawn(move || {
        let image = source.icon(&app).map(|image| {
            if image.width().max(image.height()) > ICON_PIXELS {
                image.thumbnail(ICON_PIXELS, ICON_PIXELS)
            } else {
                image
            }
        });
        let _ = events.send(Event::Icon { id: app.id, image });
    });
}
