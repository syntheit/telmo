//! Album covers for the result card: fetched in the background, kept in
//! `$XDG_CACHE_HOME/telmo/covers` so repeat results show at once.

use crate::backend::{Event, Tx};
use image::DynamicImage;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

/// What the card gives the picture, in cells. Cells are about twice as tall as
/// wide, so this is close to square.
pub const COLUMNS: u16 = 12;
pub const ROWS: u16 = 6;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 2 * 1024 * 1024;
const KEEP: usize = 30;

/// Looks the cover up on a task of the running runtime. `None` means there is
/// no picture to show; the card keeps its placeholder.
pub fn spawn(url: String, run: u64, events: Tx) {
    tokio::spawn(async move {
        let image = load(&url).await;
        let _ = events.send(Event::Cover { run, image });
    });
}

pub async fn load(url: &str) -> Option<DynamicImage> {
    let dir = cache_dir();
    let file = dir
        .as_ref()
        .map(|d| d.join(format!("{:016x}.jpg", hash(url))));
    if let Some(image) = file.as_deref().and_then(read_cached) {
        return Some(image);
    }
    let bytes = download(url).await?;
    let image = decode(bytes.clone()).await?;
    if let (Some(dir), Some(file)) = (&dir, &file) {
        store(dir, file, &bytes);
    }
    Some(image)
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("telmo").join("covers"))
}

fn hash(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

fn read_cached(file: &Path) -> Option<DynamicImage> {
    image::load_from_memory(&std::fs::read(file).ok()?).ok()
}

async fn decode(bytes: Vec<u8>) -> Option<DynamicImage> {
    tokio::task::spawn_blocking(move || image::load_from_memory(&bytes).ok())
        .await
        .ok()?
}

async fn download(url: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder().timeout(TIMEOUT).build().ok()?;
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return None;
        }
    }
    Some(bytes)
}

/// Best effort: a cover that isn't cached is only fetched again next time.
fn store(dir: &Path, file: &Path, bytes: &[u8]) {
    if std::fs::create_dir_all(dir).is_ok() && std::fs::write(file, bytes).is_ok() {
        prune(dir);
    }
}

/// Deletes everything but the newest `KEEP` files.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in files.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_newest_files() {
        let dir = std::env::temp_dir().join(format!("telmo-covers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 5 {
            let file = std::fs::File::create(dir.join(format!("{i}.jpg"))).unwrap();
            let age = Duration::from_secs((KEEP + 5 - i) as u64);
            file.set_modified(std::time::SystemTime::now() - age)
                .unwrap();
        }
        prune(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), KEEP);
        assert!(dir.join(format!("{}.jpg", KEEP + 4)).exists());
        assert!(!dir.join("0.jpg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
