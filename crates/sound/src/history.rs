//! The last songs found, kept in `$XDG_STATE_HOME/telmo/songs.json`.

use crate::song::Found;
use std::path::{Path, PathBuf};

const KEEP: usize = 10;

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("telmo").join("songs.json"))
}

pub fn load(path: &Path) -> Vec<Found> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Best effort: losing the history is not worth interrupting the result.
pub fn save(path: &Path, songs: &[Found]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(songs) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Puts `found` first. Hearing the same song again moves it up.
pub fn remember(songs: &mut Vec<Found>, found: Found) {
    songs.retain(|s| (&s.title, &s.artist) != (&found.title, &found.artist));
    songs.insert(0, found);
    songs.truncate(KEEP);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str) -> Found {
        Found {
            title: title.into(),
            artist: "A".into(),
            album: None,
            year: None,
            shazam_url: None,
            apple_music_url: None,
            spotify_search_url: String::new(),
        }
    }

    #[test]
    fn keeps_the_ten_newest_first() {
        let mut songs = Vec::new();
        for n in 0..12 {
            remember(&mut songs, song(&n.to_string()));
        }
        assert_eq!(songs.len(), 10);
        assert_eq!(songs[0].title, "11");
        assert_eq!(songs[9].title, "2");
    }

    #[test]
    fn a_repeat_moves_to_the_front() {
        let mut songs = vec![song("a"), song("b")];
        remember(&mut songs, song("b"));
        let titles: Vec<_> = songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["b", "a"]);
    }

    #[test]
    fn survives_a_round_trip_and_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("telmo-songs-{}", std::process::id()));
        let file = dir.join("songs.json");
        assert!(load(&file).is_empty());
        save(&file, &[song("a")]);
        assert_eq!(load(&file), vec![song("a")]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
