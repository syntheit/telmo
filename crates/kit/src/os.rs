//! Small desktop actions shared by the modules: copy text, open a URL.

use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::Write,
    process::{Command, Stdio},
};

/// Copy through the terminal (OSC 52), which works over ssh and mosh too, and
/// also through the local clipboard tool when there is one.
pub fn copy(text: &str) {
    print!("\x1b]52;c;{}\x07", STANDARD.encode(text));
    let _ = std::io::stdout().flush();
    let tool = if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "wl-copy"
    };
    let spawned = Command::new(tool)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else { return };
    let text = text.to_owned();
    // A thread feeds the tool and reaps it, so the UI never waits.
    std::thread::spawn(move || {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    });
}

/// Opens a page in the default browser or app.
pub fn open(url: &str) -> Result<(), String> {
    let tool = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn_reaped(
        Command::new(tool)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .map_err(|_| format!("Couldn't open the page. Is `{tool}` installed?"))
}

/// Starts `command` without waiting; a thread reaps it when it exits so no
/// zombie is left behind.
pub fn spawn_reaped(command: &mut Command) -> std::io::Result<()> {
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
