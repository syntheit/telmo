//! The user's own choices (which effect, which logo), kept between runs.
//!
//! Unlike `cache`, which only speeds up a first frame and can be deleted any
//! time, this is data the user picked. Nix never manages it, so rebuilds don't
//! reset it.

use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::OpenOptions,
    io,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

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

/// Reads the file (or its default), changes it and saves it while holding a
/// lock, so two programs changing the same file never overwrite each other.
pub fn update<T, F>(name: &str, change: F) -> io::Result<T>
where
    T: Serialize + DeserializeOwned + Default,
    F: FnOnce(&mut T),
{
    let dir = dir().ok_or_else(|| io::Error::other("no home directory"))?;
    update_in(&dir, name, change)
}

/// Like `update`, for a state directory other than the user's (tests, tools).
pub fn update_in<T, F>(dir: &Path, name: &str, change: F) -> io::Result<T>
where
    T: Serialize + DeserializeOwned + Default,
    F: FnOnce(&mut T),
{
    std::fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{name}.lock")))?;
    // SAFETY: the descriptor belongs to `lock`, which outlives the call.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut value = std::fs::read(dir.join(format!("{name}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    change(&mut value);
    write(&dir.join(format!("{name}.json")), &value)?;
    Ok(value) // closing `lock` unlocks
}

/// Writes a temp file and renames it, so a crash never leaves half a file.
pub fn save<T: Serialize>(name: &str, value: &T) -> io::Result<()> {
    write(
        &path(name).ok_or_else(|| io::Error::other("no home directory"))?,
        value,
    )
}

fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    // Per-process name: two popups saving at once must not share a temp file.
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_updates_all_land() {
        let dir = std::env::temp_dir().join(format!("telmo-state-{}", std::process::id()));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        update_in(&dir, "count", |n: &mut u32| *n += 1).expect("update");
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("worker");
        }
        let total: u32 = update_in(&dir, "count", |_| {}).expect("read");
        assert_eq!(total, 100);
        let _ = std::fs::remove_dir_all(dir);
    }
}
