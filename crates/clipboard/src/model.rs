//! Plain data: what the store keeps and what the UI shows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Link,
    Color,
    Code,
    Image,
    File,
}

/// One history item. The content itself lives in a blob file beside the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    /// Start of the text, a file name list, or a caption for images.
    pub preview: String,
    pub bytes: u64,
    pub chars: u32,
    pub words: u32,
    pub lines: u32,
    pub width: u32,
    pub height: u32,
    /// Images: `PNG`, `JPEG`. Files: a kind such as `PDF document`.
    pub format: String,
    pub files: Vec<String>,
    /// Empty when unknown.
    pub source: String,
    pub copied_at: u64,
    pub pinned: bool,
    pub hash: String,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: Kind::Text,
            preview: String::new(),
            bytes: 0,
            chars: 0,
            words: 0,
            lines: 0,
            width: 0,
            height: 0,
            format: String::new(),
            files: Vec::new(),
            source: String::new(),
            copied_at: 0,
            pinned: false,
            hash: String::new(),
        }
    }
}

/// What the backend sends to the UI whenever the history changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// Newest first.
    pub entries: Vec<Entry>,
    pub max_items: usize,
    pub max_days: u64,
}
