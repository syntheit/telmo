//! Running system tools from worker threads, always with a time limit.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program` and wait at most `limit` for it to exit.
pub fn run(program: &str, args: &[&str], limit: Duration) -> Result<Output, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run {program}: {e}."))?;
    let stdout = collect(child.stdout.take());
    let stderr = collect(child.stderr.take());
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} didn't answer in {} seconds.",
                    limit.as_secs()
                ));
            }
            Err(e) => return Err(format!("Couldn't wait for {program}: {e}.")),
        }
    };
    Ok(Output {
        ok: status.success(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn collect(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = pipe {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
    })
}

/// Quote one argument for /bin/sh.
pub fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Run shell commands as root behind the standard macOS password prompt.
pub fn admin(commands: &[Vec<String>]) -> Result<(), String> {
    let script = commands
        .iter()
        .map(|c| c.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" && ");
    let literal = script.replace('\\', "\\\\").replace('"', "\\\"");
    let apple = format!("do shell script \"{literal}\" with administrator privileges");
    // The prompt waits for a person, so give them plenty of time.
    let out = run(
        "/usr/bin/osascript",
        &["-e", &apple],
        Duration::from_secs(300),
    )?;
    if out.ok {
        return Ok(());
    }
    if out.stderr.contains("-128") {
        return Err("Cancelled.".to_string());
    }
    Err(format!("The change was refused: {}", out.stderr.trim()))
}
