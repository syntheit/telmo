//! Putting an item back on the system clipboard. Never pastes.

use std::path::PathBuf;

pub enum Payload {
    Text(String),
    /// A PNG or JPEG file from the history.
    Image(PathBuf),
    Files(Vec<String>),
}

#[cfg(target_os = "macos")]
pub fn put(payload: &Payload) -> Result<(), String> {
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_app_kit::{
        NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardWriting,
    };
    use objc2_foundation::{NSArray, NSData, NSString, NSURL};

    let pasteboard = NSPasteboard::generalPasteboard();
    // Read everything first so a bad file doesn't leave the clipboard empty.
    let image = match payload {
        Payload::Image(path) => {
            Some(std::fs::read(path).map_err(|e| format!("Can't read the picture: {e}."))?)
        }
        _ => None,
    };
    pasteboard.clearContents();
    let ok = match payload {
        Payload::Text(text) => pasteboard
            .setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString }),
        Payload::Image(path) => {
            let bytes = image.unwrap_or_default();
            let data = NSData::with_bytes(&bytes);
            let is_png = path.extension().is_none_or(|e| e == "png");
            if is_png {
                pasteboard.setData_forType(Some(&data), unsafe { NSPasteboardTypePNG })
            } else {
                pasteboard.setData_forType(Some(&data), &NSString::from_str("public.jpeg"))
            }
        }
        Payload::Files(paths) => {
            let urls: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
                .iter()
                .map(|p| {
                    ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(p)))
                })
                .collect();
            pasteboard.writeObjects(&NSArray::from_retained_slice(&urls))
        }
    };
    if ok {
        Ok(())
    } else {
        Err("macOS wouldn't take it. Try copying again.".into())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn put(payload: &Payload) -> Result<(), String> {
    use crate::wayland::to_uri;
    let (mime, bytes) = match payload {
        Payload::Text(text) => ("text/plain", text.clone().into_bytes()),
        Payload::Image(path) => {
            let mime = if path.extension().is_some_and(|e| e == "jpg") {
                "image/jpeg"
            } else {
                "image/png"
            };
            let bytes = std::fs::read(path).map_err(|e| format!("Can't read the picture: {e}."))?;
            (mime, bytes)
        }
        Payload::Files(paths) => {
            let list: Vec<String> = paths.iter().map(|p| to_uri(p)).collect();
            ("text/uri-list", list.join("\r\n").into_bytes())
        }
    };
    wl_copy(mime, &bytes)
}

#[cfg(not(target_os = "macos"))]
fn wl_copy(mime: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run wl-copy: {e}. Is wl-clipboard installed?"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(bytes)
            .map_err(|e| format!("wl-copy stopped listening: {e}."))?;
    }
    // wl-copy hands the data to a background process and exits.
    let status = child.wait().map_err(|e| format!("wl-copy failed: {e}."))?;
    if status.success() {
        Ok(())
    } else {
        Err("wl-copy failed. Is a Wayland session running?".into())
    }
}
