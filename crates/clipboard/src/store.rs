//! The history on disk. This is the only code that reads or writes it.
//!
//! `index.json` lists the entries (newest first); each entry's content is a
//! blob file next to it. Every change holds an exclusive lock on
//! `clipboard.lock` and replaces the index atomically, so the watcher, the
//! popup and the ingest command never see or leave half a write.

use crate::config::Config;
use crate::content::{self, Content, Prep, Prepared};
use crate::model::Entry;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::fd::AsRawFd,
    path::PathBuf,
    time::SystemTime,
};

type Res<T> = Result<T, String>;

const BLOB_EXTENSIONS: [&str; 4] = ["txt", "png", "jpg", "files"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stored,
    /// Nothing to keep; the reason is for a stderr note (empty: no note).
    Skipped(String),
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    config: Config,
}

impl Store {
    pub fn new(dir: PathBuf, config: Config) -> Self {
        Self { dir, config }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    pub fn config(&self) -> Config {
        self.config
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn blob_path(&self, id: &str, extension: &str) -> PathBuf {
        self.dir.join(format!("{id}.{extension}"))
    }

    /// Newest first. A missing index is an empty history.
    pub fn entries(&self) -> Res<Vec<Entry>> {
        let path = self.index_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("Can't read the history ({}): {e}.", path.display())),
        };
        serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "The history file {} is damaged: {e}. Delete it to start over.",
                path.display()
            )
        })
    }

    /// When the index last changed, for noticing other writers.
    pub fn modified(&self) -> Option<SystemTime> {
        fs::metadata(self.index_path()).ok()?.modified().ok()
    }

    pub fn ingest(&self, content: Content, source: &str, now: u64) -> Res<Outcome> {
        let prepared = match content::prepare(content)? {
            Prep::Ready(prepared) => prepared,
            Prep::Skip(reason) => return Ok(Outcome::Skipped(reason)),
            Prep::Ignore => return Ok(Outcome::Skipped(String::new())),
        };
        let config = self.config;
        self.locked(|store, entries| {
            store.add(entries, *prepared, source, now)?;
            prune(entries, &config, now);
            Ok(Outcome::Stored)
        })
    }

    /// Same content again moves the old entry to the top; otherwise it is new.
    fn add(&self, entries: &mut Vec<Entry>, prepared: Prepared, source: &str, now: u64) -> Res<()> {
        let mut entry = prepared.entry;
        if let Some(at) = entries.iter().position(|e| e.hash == entry.hash) {
            entry = entries.remove(at);
        } else {
            let path = self.blob_path(&entry.id, prepared.extension);
            fs::write(&path, &prepared.blob)
                .map_err(|e| format!("Can't save to the history ({}): {e}.", path.display()))?;
        }
        entry.copied_at = now;
        if !source.is_empty() {
            entry.source = source.to_string();
        }
        entries.insert(0, entry);
        Ok(())
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Res<()> {
        self.locked(|_, entries| {
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("That item is no longer in the history.")?;
            entry.pinned = pinned;
            Ok(())
        })
    }

    pub fn delete(&self, id: &str) -> Res<()> {
        self.locked(|_, entries| {
            entries.retain(|e| e.id != id);
            Ok(())
        })
    }

    pub fn clear(&self) -> Res<()> {
        self.locked(|_, entries| {
            entries.clear();
            Ok(())
        })
    }

    pub fn text(&self, id: &str) -> Res<String> {
        let path = self.blob_path(id, "txt");
        fs::read_to_string(&path)
            .map_err(|e| format!("Can't read the text ({}): {e}.", path.display()))
    }

    pub fn image_path(&self, entry: &Entry) -> PathBuf {
        let extension = if entry.format == "JPEG" { "jpg" } else { "png" };
        self.blob_path(&entry.id, extension)
    }

    /// Runs `change` on the index under the lock, then writes it back and
    /// removes the blobs of anything that dropped out.
    fn locked<T>(&self, change: impl FnOnce(&Store, &mut Vec<Entry>) -> Res<T>) -> Res<T> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| format!("Can't create {}: {e}.", self.dir.display()))?;
        let lock = self.lock()?;
        let mut entries = self.entries()?;
        let before: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let result = change(self, &mut entries)?;
        self.write(&entries)?;
        for id in before
            .iter()
            .filter(|id| !entries.iter().any(|e| &e.id == *id))
        {
            self.remove_blobs(id);
        }
        drop(lock);
        Ok(result)
    }

    fn lock(&self) -> Res<Lock> {
        let path = self.dir.join("clipboard.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Can't open {}: {e}.", path.display()))?;
        // SAFETY: the descriptor belongs to `file`, which outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!(
                "Can't lock the history: {}.",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Lock(file))
    }

    fn write(&self, entries: &[Entry]) -> Res<()> {
        let bytes = serde_json::to_vec_pretty(entries)
            .map_err(|e| format!("Can't encode the history: {e}."))?;
        let temp = self.dir.join("index.json.tmp");
        let written = File::create(&temp).and_then(|mut f| {
            f.write_all(&bytes)?;
            f.sync_all()
        });
        written
            .and_then(|()| fs::rename(&temp, self.index_path()))
            .map_err(|e| format!("Can't save the history: {e}."))
    }

    fn remove_blobs(&self, id: &str) {
        for extension in BLOB_EXTENSIONS {
            let _ = fs::remove_file(self.blob_path(id, extension));
        }
    }
}

/// Unlocks when dropped (closing the file releases the flock).
struct Lock(#[allow(dead_code)] File);

/// Drops unpinned items past `max_days`, then keeps the newest `max_items`
/// unpinned ones. Pins never expire and don't count.
pub fn prune(entries: &mut Vec<Entry>, config: &Config, now: u64) {
    let oldest = now.saturating_sub(config.max_days.saturating_mul(86400));
    let mut kept = 0;
    entries.retain(|e| {
        if e.pinned {
            return true;
        }
        if e.copied_at < oldest || kept >= config.max_items {
            return false;
        }
        kept += 1;
        true
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::content::tests::png;
    use crate::model::Kind;

    pub fn temp_store(name: &str, config: Config) -> Store {
        let dir = std::env::temp_dir().join(format!("telmo-clip-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::new(dir, config)
    }

    fn text(store: &Store, text: &str, now: u64) {
        store
            .ingest(Content::Text(text.into()), "Test", now)
            .unwrap();
    }

    #[test]
    fn missing_history_is_empty() {
        let store = temp_store("empty", Config::default());
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn newest_first_with_blobs() {
        let store = temp_store("order", Config::default());
        text(&store, "first", 100);
        text(&store, "second", 200);
        let entries = store.entries().unwrap();
        assert_eq!(entries[0].preview, "second");
        assert_eq!(entries[0].source, "Test");
        assert_eq!(store.text(&entries[1].id).unwrap(), "first");
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn same_content_moves_to_the_top() {
        let store = temp_store("dedup", Config::default());
        text(&store, "a", 100);
        text(&store, "b", 200);
        store
            .set_pinned(&store.entries().unwrap()[1].id, true)
            .unwrap();
        store.ingest(Content::Text("a".into()), "", 300).unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].preview, "a");
        assert_eq!(entries[0].copied_at, 300);
        assert_eq!(
            entries[0].source, "Test",
            "unknown source keeps the old one"
        );
        assert!(entries[0].pinned, "the pin survives");
        store.ingest(Content::Text("a".into()), "Zen", 400).unwrap();
        assert_eq!(store.entries().unwrap()[0].source, "Zen");
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn prunes_by_count_and_keeps_pins() {
        let config = Config {
            max_items: 2,
            max_days: 30,
        };
        let store = temp_store("count", config);
        text(&store, "one", 100);
        store
            .set_pinned(&store.entries().unwrap()[0].id, true)
            .unwrap();
        for (i, t) in ["two", "three", "four"].iter().enumerate() {
            text(&store, t, 200 + i as u64);
        }
        let previews: Vec<String> = store
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| e.preview)
            .collect();
        assert_eq!(previews, ["four", "three", "one"]);
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn prunes_by_age_but_not_pins() {
        let day = 86400;
        let mut entries = vec![
            Entry {
                id: "new".into(),
                copied_at: 100 * day,
                ..Entry::default()
            },
            Entry {
                id: "old".into(),
                copied_at: 60 * day,
                ..Entry::default()
            },
            Entry {
                id: "pin".into(),
                copied_at: 10 * day,
                pinned: true,
                ..Entry::default()
            },
        ];
        prune(
            &mut entries,
            &Config {
                max_items: 50,
                max_days: 30,
            },
            100 * day,
        );
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["new", "pin"]);
    }

    #[test]
    fn dropped_entries_lose_their_blobs() {
        let store = temp_store(
            "blobs",
            Config {
                max_items: 1,
                max_days: 30,
            },
        );
        text(&store, "old", 100);
        let old = store.entries().unwrap()[0].id.clone();
        text(&store, "new", 200);
        assert!(!store.blob_path(&old, "txt").exists());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn delete_clear_and_pin() {
        let store = temp_store("edit", Config::default());
        text(&store, "a", 100);
        text(&store, "b", 200);
        let id = store.entries().unwrap()[0].id.clone();
        store.set_pinned(&id, true).unwrap();
        assert!(store.entries().unwrap()[0].pinned);
        store.delete(&id).unwrap();
        assert_eq!(store.entries().unwrap().len(), 1);
        assert!(store.set_pinned(&id, true).is_err());
        store.clear().unwrap();
        assert!(store.entries().unwrap().is_empty());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn keeps_images_and_files() {
        let store = temp_store("kinds", Config::default());
        store.ingest(Content::Image(png(8, 4)), "", 100).unwrap();
        store
            .ingest(Content::Files(vec!["/tmp/x.pdf".into()]), "", 200)
            .unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries[0].kind, Kind::File);
        assert_eq!(entries[1].kind, Kind::Image);
        assert!(store.image_path(&entries[1]).exists());
        assert!(store.blob_path(&entries[0].id, "files").exists());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn whitespace_is_ignored() {
        let store = temp_store("blank", Config::default());
        assert_eq!(
            store.ingest(Content::Text("  \n".into()), "", 1).unwrap(),
            Outcome::Skipped(String::new())
        );
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn two_writers_do_not_lose_entries() {
        let store = temp_store(
            "threads",
            Config {
                max_items: 500,
                max_days: 30,
            },
        );
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for i in 0..15 {
                        store
                            .ingest(Content::Text(format!("writer {t} item {i}")), "", 1000 + i)
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.entries().unwrap().len(), 60);
        assert!(!store.dir().join("index.json.tmp").exists());
        let _ = fs::remove_dir_all(store.dir());
    }
}
