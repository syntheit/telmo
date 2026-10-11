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
/// zombie is left behind. It runs in a session of its own: a popup is its
/// terminal's session leader, and quitting right after would hang it up.
pub fn spawn_reaped(command: &mut Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_reaped_starts_its_own_session() {
        let file = std::env::temp_dir().join(format!("telmo-reaped-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        spawn_reaped(
            Command::new("sh")
                .arg("-c")
                .arg(format!("echo $$ > {}; sleep 2", file.display())),
        )
        .expect("spawn");
        let pid = (0..100)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(20));
                std::fs::read_to_string(&file)
                    .ok()?
                    .trim()
                    .parse::<i32>()
                    .ok()
            })
            .expect("child wrote its pid");
        let _ = std::fs::remove_file(&file);
        // SAFETY: getsid only reads the session id of a live process.
        assert_eq!(
            unsafe { libc::getsid(pid) },
            pid,
            "child leads its own session"
        );
        // SAFETY: the child is ours and still sleeping.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}
