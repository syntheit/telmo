//! Power and session actions. Commands run detached in their own process
//! group, so closing the popup can't kill them.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Lock,
    Sleep,
    Restart,
    ShutDown,
    LogOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Done(Cmd),
    /// A sentence for the user: what went wrong and what to do.
    Failed(String),
    /// `--mock` only: what would have happened.
    Note(String),
}

pub type Rx = UnboundedReceiver<Cmd>;
pub type Tx = UnboundedSender<Event>;

impl Cmd {
    fn verb(self) -> &'static str {
        match self {
            Cmd::Lock => "lock the screen",
            Cmd::Sleep => "sleep",
            Cmd::Restart => "restart",
            Cmd::ShutDown => "shut down",
            Cmd::LogOut => "log out",
        }
    }
}

/// Runs on its own OS thread; ends when the UI drops the command channel.
pub fn spawn(mock: bool, mut cmds: Rx, events: Tx) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Some(cmd) = cmds.blocking_recv() {
            let event = if mock { note(cmd) } else { run(cmd) };
            let _ = events.send(event);
        }
    })
}

fn note(cmd: Cmd) -> Event {
    Event::Note(format!("Would {}.", cmd.verb()))
}

fn run(cmd: Cmd) -> Event {
    match execute(cmd) {
        Ok(()) => Event::Done(cmd),
        Err(why) => Event::Failed(format!("Could not {}: {why}", cmd.verb())),
    }
}

fn execute(cmd: Cmd) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos(cmd);
    #[cfg(not(target_os = "macos"))]
    return linux(cmd);
}

#[cfg(target_os = "macos")]
fn macos(cmd: Cmd) -> Result<(), String> {
    let event = |code: &str| format!("tell application \"loginwindow\" to «event {code}»");
    match cmd {
        Cmd::Lock => lock_screen(),
        Cmd::Sleep => command("pmset", &["sleepnow"]),
        Cmd::Restart => command("osascript", &["-e", &event("aevtrest")]),
        Cmd::ShutDown => command("osascript", &["-e", &event("aevtshut")]),
        Cmd::LogOut => command("osascript", &["-e", &event("aevtrlgo")]),
    }
}

#[cfg(target_os = "macos")]
unsafe fn dl_error() -> Option<String> {
    let message = unsafe { libc::dlerror() };
    (!message.is_null()).then(|| {
        unsafe { std::ffi::CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    })
}

/// The private login framework is what the Lock Screen menu item uses.
#[cfg(target_os = "macos")]
fn lock_screen() -> Result<(), String> {
    use std::ffi::c_void;
    let library = c"/System/Library/PrivateFrameworks/login.framework/login";
    // SAFETY: dlopen/dlsym with valid C strings; the symbol has the signature
    // `int SACLockScreenImmediate(void)`, and we only call it when found.
    unsafe {
        let handle = libc::dlopen(library.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return Err(dl_error().unwrap_or_else(|| "macOS has no login framework here.".into()));
        }
        let symbol = libc::dlsym(handle, c"SACLockScreenImmediate".as_ptr());
        if symbol.is_null() {
            return Err(
                "this macOS version has no lock call. Use the Lock Screen menu item.".into(),
            );
        }
        let lock: extern "C" fn() -> i32 = std::mem::transmute::<*mut c_void, _>(symbol);
        match lock() {
            0 => Ok(()),
            code => Err(format!(
                "macOS refused (error {code}). Try Control-Command-Q."
            )),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn linux(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Lock => command("loginctl", &["lock-session"]),
        Cmd::Sleep => command("systemctl", &["suspend"]),
        Cmd::Restart => command("systemctl", &["reboot"]),
        Cmd::ShutDown => command("systemctl", &["poweroff"]),
        Cmd::LogOut if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() => {
            command("hyprctl", &["dispatch", "exit"])
        }
        Cmd::LogOut => {
            let session = std::env::var("XDG_SESSION_ID").map_err(|_| {
                "no login session was found. Log out from your desktop instead.".to_string()
            })?;
            command("loginctl", &["terminate-session", &session])
        }
    }
}

/// Runs a program to completion; its stderr becomes the failure sentence.
fn command(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{program} did not start ({e}). Is it installed?"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.');
    if reason.is_empty() {
        Err(format!("{program} failed. Try again."))
    } else {
        Err(format!("{reason}."))
    }
}
