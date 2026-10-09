//! Turning copied things into history entries (without touching the disk).

use crate::detect;
use crate::model::{Entry, Kind};
use std::path::Path;

pub const MAX_TEXT: usize = 1024 * 1024;
pub const MAX_IMAGE: usize = 30 * 1024 * 1024;
const PREVIEW_CHARS: usize = 2000;

/// What the watchers hand to the store.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    /// PNG or JPEG bytes.
    Image(Vec<u8>),
    /// Absolute paths.
    Files(Vec<String>),
}

/// An entry plus the bytes for its blob file.
pub struct Prepared {
    pub entry: Entry,
    pub extension: &'static str,
    pub blob: Vec<u8>,
}

pub enum Prep {
    Ready(Box<Prepared>),
    /// Valid but not worth keeping, with the reason for a stderr note.
    Skip(String),
    /// Whitespace-only text.
    Ignore,
}

pub fn prepare(content: Content) -> Result<Prep, String> {
    match content {
        Content::Text(text) => prepare_text(text),
        Content::Image(bytes) => prepare_image(bytes),
        Content::Files(paths) => prepare_files(paths),
    }
}

fn prepare_text(text: String) -> Result<Prep, String> {
    if text.is_empty() {
        return Err("The clipboard text is empty.".into());
    }
    if text.trim().is_empty() {
        return Ok(Prep::Ignore);
    }
    if text.len() > MAX_TEXT {
        return Ok(Prep::Skip(format!(
            "Skipped {} KB of text: items over 1 MB aren't kept.",
            text.len() / 1024
        )));
    }
    let (chars, words, lines) = detect::counts(&text);
    let entry = Entry {
        kind: detect::kind_of(&text),
        preview: text.chars().take(PREVIEW_CHARS).collect(),
        bytes: text.len() as u64,
        chars,
        words,
        lines,
        hash: hash("text", text.as_bytes()),
        ..Entry::default()
    };
    Ok(ready(entry, "txt", text.into_bytes()))
}

fn prepare_image(bytes: Vec<u8>) -> Result<Prep, String> {
    if bytes.is_empty() {
        return Err("The image is empty.".into());
    }
    if bytes.len() > MAX_IMAGE {
        return Ok(Prep::Skip(format!(
            "Skipped a {} MB image: items over 30 MB aren't kept.",
            bytes.len() / (1024 * 1024)
        )));
    }
    let (format, extension, label) = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Png) => (image::ImageFormat::Png, "png", "PNG"),
        Ok(image::ImageFormat::Jpeg) => (image::ImageFormat::Jpeg, "jpg", "JPEG"),
        _ => return Err("That image isn't a PNG or JPEG, so it can't be kept.".into()),
    };
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|e| format!("Couldn't read the image: {e}."))?;
    let entry = Entry {
        kind: Kind::Image,
        preview: format!("Image {width}×{height}"),
        bytes: bytes.len() as u64,
        width,
        height,
        format: label.into(),
        hash: hash("image", &bytes),
        ..Entry::default()
    };
    Ok(ready(entry, extension, bytes))
}

fn prepare_files(paths: Vec<String>) -> Result<Prep, String> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return Err("The list of files is empty.".into());
    }
    if let Some(bad) = paths.iter().find(|p| !p.starts_with('/')) {
        return Err(format!("\"{bad}\" isn't an absolute path."));
    }
    let list = paths.join("\n");
    let names: Vec<&str> = paths.iter().map(|p| file_name(p)).collect();
    let preview = match names.len() {
        1 => names[0].to_string(),
        n => format!("{} +{}", names[0], n - 1),
    };
    let sizes: Vec<std::fs::Metadata> = paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .collect();
    let entry = Entry {
        kind: Kind::File,
        preview,
        bytes: sizes.iter().filter(|m| m.is_file()).map(|m| m.len()).sum(),
        format: if paths.len() == 1 {
            file_kind(&paths[0], sizes.first().is_some_and(|m| m.is_dir()))
        } else {
            format!("{} files", paths.len())
        },
        files: paths,
        hash: hash("files", list.as_bytes()),
        ..Entry::default()
    };
    Ok(ready(entry, "files", list.into_bytes()))
}

fn ready(mut entry: Entry, extension: &'static str, blob: Vec<u8>) -> Prep {
    entry.id = entry.hash.clone();
    Prep::Ready(Box::new(Prepared {
        entry,
        extension,
        blob,
    }))
}

pub fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// "PDF document", "Folder", "File".
pub fn file_kind(path: &str, is_dir: bool) -> String {
    if is_dir {
        return "Folder".into();
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        None => "File".into(),
        Some("pdf") => "PDF document".into(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg") => "Image".into(),
        Some("txt" | "md") => "Text document".into(),
        Some("zip" | "gz" | "tar" | "tgz" | "xz" | "7z") => "Archive".into(),
        Some(other) => format!("{} file", other.to_ascii_uppercase()),
    }
}

/// FNV-1a over the kind and the bytes, as 16 hex digits. Stable across runs
/// and Rust versions, which a stored hash needs.
pub fn hash(tag: &str, bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in tag.bytes().chain([0]).chain(bytes.iter().copied()) {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn ready_entry(content: Content) -> Entry {
        match prepare(content).unwrap() {
            Prep::Ready(p) => p.entry,
            _ => panic!("expected an entry"),
        }
    }

    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn text_gets_counts_and_a_kind() {
        let entry = ready_entry(Content::Text("hello big world\nsecond".into()));
        assert_eq!((entry.chars, entry.words, entry.lines), (22, 4, 2));
        assert_eq!(entry.kind, Kind::Text);
        assert_eq!(entry.id, entry.hash);
        assert_eq!(
            ready_entry(Content::Text("https://example.com".into())).kind,
            Kind::Link
        );
    }

    #[test]
    fn empty_and_blank_text() {
        assert!(prepare(Content::Text(String::new())).is_err());
        assert!(matches!(
            prepare(Content::Text(" \n\t".into())),
            Ok(Prep::Ignore)
        ));
    }

    #[test]
    fn big_text_is_skipped() {
        let big = "a".repeat(MAX_TEXT + 1);
        assert!(matches!(prepare(Content::Text(big)), Ok(Prep::Skip(_))));
    }

    #[test]
    fn images_report_size_and_format() {
        let entry = ready_entry(Content::Image(png(30, 20)));
        assert_eq!((entry.width, entry.height), (30, 20));
        assert_eq!(entry.format, "PNG");
        assert_eq!(entry.preview, "Image 30×20");
        assert!(prepare(Content::Image(b"not an image".to_vec())).is_err());
        assert!(prepare(Content::Image(Vec::new())).is_err());
    }

    #[test]
    fn files_need_absolute_paths() {
        assert!(prepare(Content::Files(vec!["a.txt".into()])).is_err());
        assert!(prepare(Content::Files(vec!["  ".into()])).is_err());
        let entry = ready_entry(Content::Files(vec![
            "/x/invoice.pdf".into(),
            "/x/b.txt".into(),
        ]));
        assert_eq!(entry.preview, "invoice.pdf +1");
        assert_eq!(entry.format, "2 files");
        let one = ready_entry(Content::Files(vec!["/x/invoice.pdf".into()]));
        assert_eq!(one.format, "PDF document");
    }

    #[test]
    fn hash_differs_by_kind_and_content() {
        assert_eq!(hash("text", b"a"), hash("text", b"a"));
        assert_ne!(hash("text", b"a"), hash("text", b"b"));
        assert_ne!(hash("text", b"a"), hash("files", b"a"));
    }
}
