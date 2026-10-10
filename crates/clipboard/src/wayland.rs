//! Linux: `wl-paste --watch telmo-clipboard ingest-wayland`.

use crate::content::{Content, MAX_IMAGE, MAX_TEXT};
use crate::ingest;
use crate::store::Outcome;
use std::io::Read;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// PNG or JPEG, stored as they are.
    Image(String),
    Files,
    Text(String),
}

/// Which of the offered types to read: pictures first, then files, then text.
pub fn pick_type(types: &[String]) -> Option<Pick> {
    let offered = |wanted: &str| types.iter().any(|t| t.trim().eq_ignore_ascii_case(wanted));
    for image in ["image/png", "image/jpeg"] {
        if offered(image) {
            return Some(Pick::Image(image.into()));
        }
    }
    if offered("text/uri-list") {
        return Some(Pick::Files);
    }
    ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
        .into_iter()
        .find(|t| offered(t))
        .map(|t| Pick::Text(t.into()))
}

/// `file:///a%20b` lines to `/a b`. Comments and other schemes are dropped.
pub fn parse_uri_list(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| line.trim().strip_prefix("file://"))
        .map(|rest| rest.strip_prefix("localhost").unwrap_or(rest))
        .map(percent_decode)
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `/a b` to `file:///a%20b`, for handing files back to wl-copy.
#[cfg(any(not(target_os = "macos"), test))]
pub fn to_uri(path: &str) -> String {
    let mut out = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn ingest() -> Result<Outcome, String> {
    // wl-paste --watch pipes the clipboard to our stdin; we read it ourselves
    // by type, but must drain the pipe so wl-paste never sees it close early.
    let mut sink = Vec::new();
    let _ = std::io::stdin()
        .take(MAX_IMAGE as u64)
        .read_to_end(&mut sink);
    drop(sink);

    let listed = wl_paste(&["--list-types"])?;
    let types: Vec<String> = String::from_utf8_lossy(&listed)
        .lines()
        .map(String::from)
        .collect();
    let Some(pick) = pick_type(&types) else {
        return Ok(Outcome::Skipped(String::new()));
    };
    let store = ingest::open_store()?;
    let source = active_window_class();
    let content = match pick {
        Pick::Image(mime) => Content::Image(wl_paste(&["--type", &mime, "--no-newline"])?),
        Pick::Files => {
            let list = wl_paste(&["--type", "text/uri-list", "--no-newline"])?;
            Content::Files(parse_uri_list(&String::from_utf8_lossy(&list)))
        }
        Pick::Text(mime) => {
            let bytes = wl_paste(&["--type", &mime, "--no-newline"])?;
            if bytes.len() > MAX_TEXT {
                return Ok(Outcome::Skipped("Skipped text over 1 MB.".into()));
            }
            Content::Text(String::from_utf8_lossy(&bytes).into_owned())
        }
    };
    store.ingest(content, &source, telmo_kit::time::unix_now())
}

fn wl_paste(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("wl-paste")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run wl-paste: {e}. Is wl-clipboard installed?"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err("wl-paste failed. Is a Wayland session running?".into())
    }
}

/// The focused window's class, or empty when Hyprland can't say.
fn active_window_class() -> String {
    let Ok(output) = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return String::new();
    };
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|json| json["class"].as_str().map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pictures_win_over_everything() {
        let offered = types(&["text/html", "image/png", "text/plain"]);
        assert_eq!(pick_type(&offered), Some(Pick::Image("image/png".into())));
        assert_eq!(
            pick_type(&types(&["image/jpeg", "text/plain"])),
            Some(Pick::Image("image/jpeg".into()))
        );
    }

    #[test]
    fn files_before_text() {
        let offered = types(&["text/uri-list", "text/plain", "text/plain;charset=utf-8"]);
        assert_eq!(pick_type(&offered), Some(Pick::Files));
    }

    #[test]
    fn text_prefers_utf8() {
        let offered = types(&["UTF8_STRING", "text/plain", "text/plain;charset=utf-8"]);
        assert_eq!(
            pick_type(&offered),
            Some(Pick::Text("text/plain;charset=utf-8".into()))
        );
        assert_eq!(
            pick_type(&types(&["UTF8_STRING"])),
            Some(Pick::Text("UTF8_STRING".into()))
        );
    }

    #[test]
    fn nothing_usable() {
        assert_eq!(pick_type(&types(&["image/webp", "text/html"])), None);
        assert_eq!(pick_type(&[]), None);
    }

    #[test]
    fn uri_lists() {
        let list =
            "# comment\nfile:///home/me/My%20Doc.pdf\r\nhttps://example.com\nfile:///tmp/b\n";
        assert_eq!(parse_uri_list(list), ["/home/me/My Doc.pdf", "/tmp/b"]);
        assert_eq!(
            to_uri("/home/me/My Doc.pdf"),
            "file:///home/me/My%20Doc.pdf"
        );
        assert_eq!(parse_uri_list(&to_uri("/a/ü b")), ["/a/ü b"]);
    }
}
