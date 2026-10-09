//! `--mock`: the history from the approved mockups, kept in memory.

use super::Source;
use crate::detect;
use crate::model::{Entry, Kind, Snapshot};
use image::DynamicImage;
use std::{sync::Mutex, time::SystemTime};

const MIN: u64 = 60;
const HOUR: u64 = 3600;
const DAY: u64 = 86400;

pub struct Mock {
    entries: Mutex<Vec<Entry>>,
}

impl Mock {
    pub fn new(now: u64) -> Self {
        Self {
            entries: Mutex::new(entries(now)),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn text(id: &str, kind: Kind, content: &str, ago: u64, source: &str, now: u64) -> Entry {
    let (chars, words, lines) = detect::counts(content);
    Entry {
        id: id.into(),
        kind,
        preview: content.into(),
        bytes: content.len() as u64,
        chars,
        words,
        lines,
        source: source.into(),
        copied_at: now - ago,
        ..Entry::default()
    }
}

fn picture(
    id: &str,
    caption: &str,
    size: (u32, u32),
    kb: u64,
    ago: u64,
    source: &str,
    now: u64,
) -> Entry {
    Entry {
        id: id.into(),
        kind: Kind::Image,
        preview: caption.into(),
        bytes: kb * 1024,
        width: size.0,
        height: size.1,
        format: "PNG".into(),
        source: source.into(),
        copied_at: now - ago,
        ..Entry::default()
    }
}

/// The 13 items of the mockups, newest first.
pub fn entries(now: u64) -> Vec<Entry> {
    let file = Entry {
        id: "invoice".into(),
        kind: Kind::File,
        preview: "invoice-2026-10.pdf".into(),
        bytes: 184 * 1024,
        format: "PDF document".into(),
        files: vec!["/Users/daniel/Downloads/invoice-2026-10.pdf".into()],
        source: "Finder".into(),
        copied_at: now - 3 * HOUR,
        ..Entry::default()
    };
    let sudo = "sudo darwin-rebuild switch --flake ~/nix#swift --substituters\n  'https://cache.nixos.org'";
    vec![
        text("sudo", Kind::Text, sudo, 2 * MIN + 10, "Ghostty", now),
        picture(
            "shot",
            "Screenshot 1440×900",
            (1440, 900),
            412,
            5 * MIN + 10,
            "Screenshot",
            now,
        ),
        text(
            "link",
            Kind::Link,
            "https://github.com/syntheit/telmo/pull/12",
            18 * MIN,
            "Zen",
            now,
        ),
        text("color", Kind::Color, "#7aa2f7", HOUR + 5 * MIN, "Zen", now),
        text(
            "meeting",
            Kind::Text,
            "Meeting moved to 3:30, same room as last week",
            HOUR + 30 * MIN,
            "Messages",
            now,
        ),
        file,
        text(
            "code",
            Kind::Code,
            "fn main() { println!(\"hello\"); }",
            5 * HOUR,
            "Zed",
            now,
        ),
        text(
            "secret",
            Kind::Text,
            "hunter2-correct-horse-battery-staple",
            DAY + 2 * HOUR,
            "1Password",
            now,
        ),
        picture(
            "heic",
            "IMG_4021.HEIC 4032×3024",
            (4032, 3024),
            2800,
            DAY + 6 * HOUR,
            "Photos",
            now,
        ),
        text(
            "news",
            Kind::Link,
            "news.ycombinator.com/item?id=41234567",
            2 * DAY + HOUR,
            "Zen",
            now,
        ),
        text(
            "email",
            Kind::Text,
            "daniel@matv.io",
            3 * DAY + HOUR,
            "Mail",
            now,
        ),
        Entry {
            pinned: true,
            ..text(
                "ssh",
                Kind::Text,
                "ssh daniel@mantle -t tmux attach",
                9 * DAY,
                "Ghostty",
                now,
            )
        },
        Entry {
            pinned: true,
            ..text(
                "phone",
                Kind::Text,
                "+54 9 11 5555 0142",
                20 * DAY,
                "Contacts",
                now,
            )
        },
    ]
}

/// A gradient with a hard edge, so scaling mistakes show.
pub fn test_image() -> DynamicImage {
    let image = image::RgbaImage::from_fn(320, 200, |x, y| {
        if x < 160 && y < 100 {
            image::Rgba([0x7a, 0xa2, 0xf7, 255])
        } else {
            image::Rgba([(x * 255 / 320) as u8, (y * 255 / 200) as u8, 0x9a, 255])
        }
    });
    DynamicImage::ImageRgba8(image)
}

impl Source for Mock {
    fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(Snapshot {
            entries: self.entries().clone(),
            max_items: 50,
            max_days: 30,
        })
    }

    fn version(&self) -> Option<SystemTime> {
        None
    }

    fn pin(&self, id: &str, pinned: bool) -> Result<(), String> {
        if let Some(entry) = self.entries().iter_mut().find(|e| e.id == id) {
            entry.pinned = pinned;
        }
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.entries().retain(|e| e.id != id);
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        self.entries().clear();
        Ok(())
    }

    fn text(&self, id: &str) -> Result<String, String> {
        self.entries()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.preview.clone())
            .ok_or_else(|| "That item is no longer in the history.".into())
    }

    fn image(&self, _id: &str) -> Result<DynamicImage, String> {
        Ok(test_image())
    }

    fn copy(&self, id: &str, text: Option<&str>) -> Result<Option<String>, String> {
        let what = match text {
            Some(text) => text.to_string(),
            None => self
                .entries()
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.preview.lines().next().unwrap_or("").to_string())
                .ok_or("That item is no longer in the history.")?,
        };
        Ok(Some(format!("Would copy {what}")))
    }
}
