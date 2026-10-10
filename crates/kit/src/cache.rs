//! Last-known snapshots, so a popup can draw real-looking data on its first
//! frame while the backend is still starting.

use serde::{Serialize, de::DeserializeOwned};
use std::path::PathBuf;

fn path(name: &str) -> Option<PathBuf> {
    Some(
        crate::dirs::cache()?
            .join("telmo")
            .join(format!("{name}.json")),
    )
}

pub fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    let bytes = std::fs::read(path(name)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Best effort: a failed write only costs a slower first frame next time.
pub fn save<T: Serialize>(name: &str, value: &T) {
    let Some(path) = path(name) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(value) {
        let _ = std::fs::write(path, bytes);
    }
}
