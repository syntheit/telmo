//! `telmo popup`: macOS talks to the host app, Linux drives Hyprland.

#[cfg(target_os = "macos")]
pub use macos::{host_command, toggle};

#[cfg(not(target_os = "macos"))]
pub use linux::toggle;

#[cfg(not(target_os = "macos"))]
pub fn host_command(_command: &str) -> Result<String, String> {
    Err("the host app only exists on macOS".into())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::Duration;
    use std::{env, thread};

    fn socket_path() -> Result<PathBuf, String> {
        if let Some(path) = env::var_os("TELMO_SOCKET") {
            return Ok(PathBuf::from(path));
        }
        let home = env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
    }

    fn send(command: &str) -> std::io::Result<String> {
        let path = socket_path().map_err(std::io::Error::other)?;
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(format!("{command}\n").as_bytes())?;
        // The host closes the connection after replying, and a reply may span lines.
        let mut reply = String::new();
        stream.read_to_string(&mut reply)?;
        Ok(reply.trim_end_matches('\n').to_string())
    }

    /// Sends a command, starting the host app first if it is not running.
    pub fn host_command(command: &str) -> Result<String, String> {
        if let Ok(reply) = send(command) {
            return Ok(reply);
        }
        start_host()?;
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(200));
            if let Ok(reply) = send(command) {
                return Ok(reply);
            }
        }
        Err(
            "the Telmo host app did not start; open Telmo.app once and check that it stays running"
                .into(),
        )
    }

    fn start_host() -> Result<(), String> {
        let beside = env::current_exe().ok().and_then(|exe| {
            let dir = exe.parent()?.to_path_buf();
            [dir.join("../Applications/Telmo.app"), dir.join("Telmo.app")]
                .into_iter()
                .find(|p| p.exists())
        });
        let mut open = Command::new("open");
        open.arg("-g");
        match beside {
            Some(app) => open.arg(app),
            None => open.args(["-a", "Telmo"]),
        };
        let status = open
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("cannot run open: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Telmo.app is not installed (could not open it)".into())
        }
    }

    pub fn toggle(module: &str) -> Result<(), String> {
        let reply = host_command(&format!("toggle {module}"))?;
        if reply == "ok" {
            Ok(())
        } else {
            Err(reply.trim_start_matches("error ").to_string())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod linux {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    use crate::find_module;

    /// Arguments that set the window class and introduce the command, per terminal.
    fn terminal_args(terminal: &str, class: &str) -> Vec<String> {
        let name = terminal.rsplit('/').next().unwrap_or(terminal);
        let args: &[&str] = match name {
            "ghostty" => &[&format!("--class={class}"), "-e"],
            "foot" => &["--app-id", class],
            "wezterm" => &["start", "--class", class, "--"],
            "kitty" => &["--class", class],
            _ => &["--class", class, "-e"], // alacritty and unknown terminals
        };
        args.iter().map(|a| a.to_string()).collect()
    }

    fn find_terminal() -> Result<String, String> {
        if let Some(terminal) = std::env::var("TELMO_TERMINAL")
            .ok()
            .filter(|t| !t.is_empty())
        {
            return Ok(terminal);
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        ["ghostty", "kitty", "foot", "alacritty", "wezterm"]
            .into_iter()
            .find(|t| std::env::split_paths(&path).any(|dir| dir.join(t).is_file()))
            .map(String::from)
            .ok_or_else(|| "no terminal found; set TELMO_TERMINAL or install ghostty, kitty, foot, alacritty or wezterm".into())
    }

    /// Address of an existing popup window for this class, if any.
    fn existing_window(class: &str) -> Result<Option<String>, String> {
        let out = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .map_err(|e| format!("cannot run hyprctl: {e}"))?;
        if !out.status.success() {
            return Err("hyprctl clients failed; is Hyprland running?".into());
        }
        let clients: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("unreadable hyprctl output: {e}"))?;
        let found = clients
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["class"] == class);
        Ok(found.and_then(|c| c["address"].as_str()).map(String::from))
    }

    pub fn toggle(module: &str) -> Result<(), String> {
        let class = format!("telmo.{module}");
        if let Some(address) = existing_window(&class)? {
            let status = Command::new("hyprctl")
                .args(["dispatch", "closewindow", &format!("address:{address}")])
                .status();
            return status
                .map_err(|e| format!("cannot run hyprctl: {e}"))
                .map(|_| ());
        }
        let binary = find_module(module)
            .ok_or_else(|| format!("unknown module '{module}'; run `telmo list`"))?;
        let terminal = find_terminal()?;
        Command::new(&terminal)
            .args(terminal_args(&terminal, &class))
            .arg(binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("cannot start {terminal}: {e}"))?;
        Ok(())
    }
}
