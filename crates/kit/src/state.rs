//! The user's own choices (which effect, which logo), kept between runs.
//!
//! Unlike `cache`, which only speeds up a first frame and can be deleted any
//! time, this is data the user picked. Nix never manages it, so rebuilds don't
//! reset it.

use serde::{Serialize, de::DeserializeOwned};
use std::{io, path::PathBuf};

/// `~/.local/state/telmo` (or under `$XDG_STATE_HOME`).
pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("telmo"))
}

fn path(name: &str) -> Option<PathBuf> {
    Some(dir()?.join(format!("{name}.json")))
}

pub fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    let bytes = std::fs::read(path(name)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Writes a temp file and renames it, so a crash never leaves half a file.
pub fn save<T: Serialize>(name: &str, value: &T) -> io::Result<()> {
    let path = path(name).ok_or_else(|| io::Error::other("no home directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    // Per-process name: two popups saving at once must not share a temp file.
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, &path)
}
