//! macOS: app bundles on disk, NSWorkspace for who is running, `open` to
//! launch, Telmo.app for icons, NSPasteboard to copy.

use super::{Source, bundle, sys};
use crate::commands::Action;
use crate::model::AppEntry;
use image::DynamicImage;
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSPasteboard, NSPasteboardTypeString, NSWorkspace,
};
use objc2_foundation::NSString;
use std::{
    collections::HashSet,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

pub struct Real;

fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TELMO_SOCKET") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
}

/// One request line to Telmo.app and its reply. The icon is drawn by the
/// host (it has AppKit's icon cache); it answers with the PNG's path.
fn ask_host(command: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket_path()?).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    stream.write_all(format!("{command}\n").as_bytes()).ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    Some(reply.trim_end_matches('\n').to_string())
}

impl Source for Real {
    fn scan(&self) -> Result<Vec<AppEntry>, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is not set")?;
        Ok(bundle::scan(
            &bundle::search_dirs(&home),
            &bundle::extra_bundles(),
        ))
    }

    fn running(&self, _apps: &[AppEntry]) -> HashSet<String> {
        let mut ids = HashSet::new();
        for app in NSWorkspace::sharedWorkspace().runningApplications().iter() {
            if app.activationPolicy() != NSApplicationActivationPolicy::Regular
                || app.isTerminated()
            {
                continue;
            }
            if let Some(id) = app.bundleIdentifier() {
                ids.insert(id.to_string());
            }
            // Apps without an identifier are listed by path.
            if let Some(path) = app.bundleURL().and_then(|url| url.path()) {
                ids.insert(path.to_string());
            }
        }
        ids
    }

    fn icon(&self, app: &AppEntry) -> Option<DynamicImage> {
        let reply = ask_host(&format!("icon {}", app.path))?;
        if reply.starts_with("error") || reply.is_empty() {
            return None;
        }
        image::open(reply).ok()
    }

    fn open(&self, app: &AppEntry) -> Result<Option<String>, String> {
        // `open -a` starts the app, or brings it forward when it runs.
        let mut child = Command::new("open")
            .arg("-a")
            .arg(&app.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't run open: {e}."))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(None)
    }

    fn copy(&self, text: &str) -> Result<Option<String>, String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: a plain constant of AppKit.
        let kind = unsafe { NSPasteboardTypeString };
        if pasteboard.setString_forType(&NSString::from_str(text), kind) {
            Ok(None)
        } else {
            Err("macOS wouldn't take it. Try copying again.".into())
        }
    }

    fn open_url(&self, url: &str) -> Result<Option<String>, String> {
        telmo_kit::os::open(url).map(|()| None)
    }

    fn run(&self, action: &Action) -> Result<Option<String>, String> {
        sys::run_action(action)
    }

    fn clock_installed(&self) -> bool {
        sys::clock_installed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only reads NSWorkspace's list; nothing is launched or touched.
    #[test]
    fn the_finder_is_always_running() {
        let running = Real.running(&[]);
        assert!(running.contains("com.apple.finder"), "{running:?}");
    }
}
