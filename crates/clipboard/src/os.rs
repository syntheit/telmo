//! Opening and revealing things.

use std::process::{Command, Stdio};

/// A link the browser can open: bare domains get `https://`.
pub fn link_target(text: &str) -> String {
    let text = text.trim();
    if text.contains("://") || text.to_ascii_lowercase().starts_with("mailto:") {
        text.to_string()
    } else {
        format!("https://{text}")
    }
}

/// Shows a file in the file manager. macOS selects it in Finder; elsewhere
/// the folder opens.
pub fn reveal(path: &str) -> Result<(), String> {
    let (tool, argument) = if cfg!(target_os = "macos") {
        ("open", vec!["-R".to_string(), path.to_string()])
    } else {
        let folder = std::path::Path::new(path)
            .parent()
            .map_or_else(|| "/".to_string(), |p| p.display().to_string());
        ("xdg-open", vec![folder])
    };
    let mut child = Command::new(tool)
        .args(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("Couldn't show the file. Is `{tool}` installed?"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets() {
        assert_eq!(link_target("example.com/a"), "https://example.com/a");
        assert_eq!(link_target(" http://x.org "), "http://x.org");
        assert_eq!(link_target("mailto:a@b.co"), "mailto:a@b.co");
    }
}
