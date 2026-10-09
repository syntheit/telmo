//! The background rebuild.
//!
//! `u` runs `sudo telmo-system rebuild-run --as-root ...` in the popup's own
//! terminal, so Touch ID or the password prompt works. As root that process
//! only validates its arguments and starts `rebuild-run --child ...` in a new
//! session, then exits and hands the terminal back. The child runs the
//! configured command (already root), follows its output and keeps
//! `rebuild.json` and `rebuild.log` in the popup's state directory up to date,
//! owned by the user. The popup only reads them.

use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STATE: &str = "rebuild";
const PROFILE: &str = "/nix/var/nix/profiles/system";
const WRITE_EVERY: Duration = Duration::from_millis(500);

pub const NOT_CONFIGURED: &str = "Set programs.telmo.system.rebuild to rebuild from here.";
pub const ALREADY_RUNNING: &str = "A rebuild is already running.";
pub const AUTH_FAILED: &str = if cfg!(target_os = "macos") {
    "Touch ID or the password didn't go through. Press u to try again."
} else {
    "The password didn't go through. Press u to try again."
};
const FAILED: &str = "The rebuild failed. Press L to see the log.";
const STOPPED: &str = "The rebuild stopped unexpectedly.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Ok,
    Failed,
}

/// Which part of the rebuild the output says is under way.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Nothing announced yet.
    #[default]
    Evaluating,
    Downloading,
    Building,
    Activating,
}

/// The contents of `rebuild.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    pub pid: u32,
    /// Unix seconds.
    pub started: u64,
    pub finished: Option<u64>,
    pub built: u32,
    pub to_build: u32,
    pub fetched: u32,
    pub to_fetch: u32,
    /// Absent in files from older versions, which count as evaluating.
    #[serde(default)]
    pub phase: Phase,
    pub last_line: String,
    pub error: Option<String>,
    pub generation: Option<u32>,
}

impl Status {
    pub fn running(pid: u32, started: u64, last_line: &str) -> Self {
        Self {
            state: State::Running,
            pid,
            started,
            finished: None,
            built: 0,
            to_build: 0,
            fetched: 0,
            to_fetch: 0,
            phase: Phase::Evaluating,
            last_line: last_line.into(),
            error: None,
            generation: None,
        }
    }

    fn failed(&self, error: &str, now: u64) -> Self {
        Self {
            state: State::Failed,
            finished: Some(now),
            error: Some(error.into()),
            ..self.clone()
        }
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the progress line says until the command prints something.
pub const STARTING: &str = "starting…";

pub fn log_path() -> Option<PathBuf> {
    Some(telmo_kit::state::dir()?.join("rebuild.log"))
}

fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A run that says "running" but whose process is gone counts as failed.
fn check_alive(status: Status) -> Status {
    if status.state == State::Running && !pid_alive(status.pid) {
        return status.failed(STOPPED, unix_now());
    }
    status
}

/// The current state file, if there is one.
pub fn read() -> Option<Status> {
    telmo_kit::state::load::<Status>(STATE).map(check_alive)
}

fn sudo_program() -> &'static str {
    if cfg!(target_os = "macos") {
        "/usr/bin/sudo"
    } else {
        // NixOS keeps the setuid sudo in /run/wrappers/bin, which PATH finds.
        "sudo"
    }
}

/// The command `u` runs in the popup's terminal. Fails when a rebuild is
/// already running or the popup can't say where its files live.
pub fn sudo_command(configured: &[String], host: &str) -> Result<Command, String> {
    if read().is_some_and(|s| s.state == State::Running) {
        return Err(ALREADY_RUNNING.into());
    }
    let exe = std::env::current_exe()
        .map_err(|_| "Couldn't find the telmo-system program to run the rebuild.".to_string())?;
    let dir = telmo_kit::state::dir()
        .ok_or("Couldn't find the telmo state directory to keep the rebuild log in.")?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Couldn't create {} ({e}).", dir.display()))?;
    let program = configured
        .first()
        .ok_or("The rebuild command is empty. Check programs.telmo.system.rebuild.")?;
    // Root's PATH is not the user's, so hand over an absolute path.
    let program = find_program(program).ok_or_else(|| {
        format!("Couldn't find `{program}`. Check programs.telmo.system.rebuild.")
    })?;
    // SAFETY: getuid and getgid cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let mut command = Command::new(sudo_program());
    command
        .arg("-p")
        .arg(format!("Password to rebuild {host}: "))
        .arg(exe)
        .args(["rebuild-run", "--as-root", "--state-dir"])
        .arg(dir)
        .args(["--owner", &format!("{uid}:{gid}"), "--", &program])
        .args(&configured[1..]);
    Ok(command)
}

fn find_program(name: &str) -> Option<String> {
    if name.contains('/') {
        return Some(name.into());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|found| found.to_string_lossy().into_owned())
}

/// Counts of builds and fetches and the current phase, read from Nix's output.
#[derive(Debug, Default, PartialEq)]
pub struct Progress {
    pub built: u32,
    pub to_build: u32,
    pub fetched: u32,
    pub to_fetch: u32,
    pub phase: Phase,
}

impl Progress {
    pub fn feed(&mut self, line: &str) {
        let line = line.trim();
        if self.is_activation(line) {
            self.phase = Phase::Activating;
        } else if let Some(n) = announced(line, "derivation", "will be built") {
            self.to_build = n;
        } else if let Some(n) = announced(line, "path", "will be fetched") {
            self.to_fetch = n;
            self.downloading();
        } else if line.starts_with("building '") {
            self.built += 1;
            // Nix interleaves builds and fetches; once building, never back.
            if self.phase != Phase::Activating {
                self.phase = Phase::Building;
            }
        } else if line.starts_with("copying path '") {
            self.fetched += 1;
            self.downloading();
        }
    }

    /// What the switch prints when the build is done and the new system goes live:
    /// `nixos-rebuild` says `activating the configuration...` and nix-darwin's activation script,
    /// like NixOS's, starts with `setting up ...` lines (`/Applications/Nix Apps`, `/etc`, ...).
    /// Those only count once every announced build has started, so a builder that happens to
    /// print one can't end the build early.
    fn is_activation(&self, line: &str) -> bool {
        line == "activating the configuration..."
            || (line.starts_with("setting up ") && self.built >= self.to_build)
    }

    fn downloading(&mut self) {
        if self.phase == Phase::Evaluating {
            self.phase = Phase::Downloading;
        }
    }
}

/// `these 12 derivations will be built:` -> 12; `this derivation will be built:` -> 1.
fn announced(line: &str, noun: &str, tail: &str) -> Option<u32> {
    let (count, rest) = match line.strip_prefix("these ") {
        Some(rest) => rest.split_once(' ')?,
        None => ("1", line.strip_prefix("this ")?),
    };
    let rest = rest.strip_prefix(noun)?;
    let rest = rest.strip_prefix('s').unwrap_or(rest).trim_start();
    rest.starts_with(tail).then(|| count.parse().ok())?
}

/// `system-279-link` -> 279.
fn parse_generation(link: &str) -> Option<u32> {
    link.strip_prefix("system-")?
        .strip_suffix("-link")?
        .parse()
        .ok()
}

fn current_generation() -> Option<u32> {
    let target = std::fs::read_link(PROFILE).ok()?;
    parse_generation(target.file_name()?.to_str()?)
}

fn is_root() -> bool {
    // SAFETY: geteuid cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Which user the files in the state directory belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    uid: u32,
    gid: u32,
}

impl Owner {
    fn parse(text: &str) -> Option<Self> {
        let (uid, gid) = text.split_once(':')?;
        Some(Self {
            uid: uid.parse().ok()?,
            gid: gid.parse().ok()?,
        })
    }

    /// Hands a file to the user. Only root can; anyone else already owns theirs.
    fn give(self, path: &Path) {
        if !is_root() {
            return;
        }
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return;
        };
        // SAFETY: `path` is a valid NUL-terminated string.
        unsafe { libc::chown(path.as_ptr(), self.uid, self.gid) };
    }
}

/// One rebuild to run.
pub struct Job {
    pub argv: Vec<String>,
    pub waiting: String,
    /// Smallest gap between two state-file writes while output streams in.
    pub write_every: Duration,
    pub dir: PathBuf,
    pub owner: Owner,
}

impl Job {
    fn new(argv: Vec<String>, dir: PathBuf, owner: Owner) -> Self {
        Self {
            argv,
            waiting: STARTING.into(),
            write_every: WRITE_EVERY,
            dir,
            owner,
        }
    }

    fn log_file(&self) -> Option<File> {
        let path = self.dir.join("rebuild.log");
        let file = File::create(&path).ok()?;
        self.owner.give(&path);
        Some(file)
    }
}

fn save(job: &Job, status: &Status, observe: &mut dyn FnMut(&Status)) {
    // Nobody can be told about a failed write here; the log still has the output.
    let _ = write_status(job, status);
    observe(status);
}

/// Writes a temp file and renames it, so the popup never reads half a file.
fn write_status(job: &Job, status: &Status) -> std::io::Result<()> {
    let path = job.dir.join(format!("{STATE}.json"));
    let temp = job
        .dir
        .join(format!("{STATE}.json.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(status).map_err(std::io::Error::other)?;
    std::fs::write(&temp, bytes)?;
    job.owner.give(&temp);
    std::fs::rename(&temp, &path)
}

/// Runs the job to its end, keeping the state file and log current.
/// `observe` sees every status that is written.
pub fn run(job: &Job, observe: &mut dyn FnMut(&Status)) -> Status {
    let mut status = Status::running(std::process::id(), unix_now(), &job.waiting);
    let _ = std::fs::create_dir_all(&job.dir);
    job.owner.give(&job.dir);
    save(job, &status, observe);
    let mut log = job.log_file();
    let code = match follow(job, &mut status, &mut log, observe) {
        Ok(code) => code,
        Err(message) => {
            if let Some(log) = log.as_mut() {
                let _ = writeln!(log, "{message}");
            }
            None
        }
    };
    status.finished = Some(unix_now());
    if code == Some(0) {
        status.state = State::Ok;
        status.generation = current_generation();
    } else {
        status.state = State::Failed;
        status.error = Some(FAILED.into());
    }
    save(job, &status, observe);
    status
}

/// Starts the command and follows its output. Returns the exit code.
fn follow(
    job: &Job,
    status: &mut Status,
    log: &mut Option<File>,
    observe: &mut dyn FnMut(&Status),
) -> Result<Option<i32>, String> {
    let (reader, writer) =
        std::io::pipe().map_err(|e| format!("Couldn't read the output ({e})."))?;
    let stderr = writer
        .try_clone()
        .map_err(|e| format!("Couldn't read the output ({e})."))?;
    let (program, args) = job.argv.split_first().ok_or("Empty command.")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Couldn't run {program}: {e}"))?;

    let mut progress = Progress::default();
    let mut last_write = Instant::now();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if let Some(log) = log.as_mut() {
            let _ = writeln!(log, "{line}");
        }
        progress.feed(&line);
        if !line.trim().is_empty() {
            status.last_line = line.trim().to_string();
        }
        status.built = progress.built;
        status.to_build = progress.to_build;
        status.fetched = progress.fetched;
        status.to_fetch = progress.to_fetch;
        status.phase = progress.phase;
        if last_write.elapsed() >= job.write_every {
            last_write = Instant::now();
            save(job, status, observe);
        }
    }
    let code = child.wait().map_err(|e| format!("Lost the rebuild: {e}"))?;
    Ok(code.code())
}

/// The sentence the popup's footer (and on Linux the notification) shows at the end.
pub fn summary(status: &Status, host: &str) -> String {
    match (status.state, status.generation) {
        (State::Ok, Some(generation)) => format!("{host} is on generation {generation}"),
        (State::Ok, None) => format!("{host} is rebuilt"),
        _ => status.error.clone().unwrap_or_else(|| FAILED.into()),
    }
}

/// Linux has no notch pill, so the end of a rebuild that may have run with the
/// popup closed is announced as a desktop notification, in the user's session.
#[cfg(target_os = "linux")]
fn notify(text: &str, owner: Owner) {
    // Nix bakes in libnotify's path; root's PATH may not have notify-send.
    let notify_send = option_env!("TELMO_NOTIFY_SEND").unwrap_or("notify-send");
    let mut command;
    if is_root() {
        command = Command::new("sudo");
        command
            .args(["-u", &format!("#{}", owner.uid), "env"])
            .arg(format!("XDG_RUNTIME_DIR=/run/user/{}", owner.uid))
            .arg(format!(
                "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{}/bus",
                owner.uid
            ))
            .arg(notify_send);
    } else {
        command = Command::new(notify_send);
    }
    let _ = command
        .args(["--app-name=telmo", "telmo", text])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    AsRoot,
    Child,
}

/// The arguments after `rebuild-run`.
#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    mode: Mode,
    dir: PathBuf,
    owner: Owner,
    argv: Vec<String>,
}

const USAGE: &str = "Usage: telmo-system rebuild-run (--as-root|--child) --state-dir DIR --owner UID:GID -- COMMAND...";

fn parse(args: &[String]) -> Result<Invocation, String> {
    let mut mode = None;
    let mut dir = None;
    let mut owner = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--as-root" => mode = Some(Mode::AsRoot),
            "--child" => mode = Some(Mode::Child),
            "--state-dir" => dir = args.next().map(PathBuf::from),
            "--owner" => owner = args.next().and_then(|o| Owner::parse(o)),
            "--" => break,
            _ => return Err(USAGE.into()),
        }
    }
    let argv: Vec<String> = args.cloned().collect();
    match (mode, dir, owner) {
        (Some(mode), Some(dir), Some(owner)) if dir.is_absolute() && !argv.is_empty() => {
            Ok(Invocation {
                mode,
                dir,
                owner,
                argv,
            })
        }
        _ => Err(USAGE.into()),
    }
}

/// Starts the `--child` copy of this program in its own session.
fn spawn_child(invocation: &Invocation) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Couldn't find myself ({e})."))?;
    let mut command = Command::new(exe);
    command
        .args(["rebuild-run", "--child", "--state-dir"])
        .arg(&invocation.dir)
        .args([
            "--owner",
            &format!("{}:{}", invocation.owner.uid, invocation.owner.gid),
            "--",
        ])
        .args(&invocation.argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe. A new session keeps the rebuild
    // alive when the popup's terminal closes and sends SIGHUP.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Couldn't start the rebuild ({e})."))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Entry point of `telmo-system rebuild-run`; `args` follow the subcommand.
pub fn main(args: &[String]) -> ExitCode {
    let invocation = match parse(args) {
        Ok(invocation) => invocation,
        Err(sentence) => {
            eprintln!("telmo-system: {sentence}");
            return ExitCode::FAILURE;
        }
    };
    if invocation.mode == Mode::AsRoot {
        return match spawn_child(&invocation) {
            Ok(()) => ExitCode::SUCCESS,
            Err(sentence) => {
                eprintln!("telmo-system: {sentence}");
                ExitCode::FAILURE
            }
        };
    }
    let job = Job::new(invocation.argv, invocation.dir, invocation.owner);
    let status = run(&job, &mut |_| {});
    #[cfg(target_os = "linux")]
    notify(
        &summary(&status, &crate::app::host_name()),
        invocation.owner,
    );
    if status.state == State::Ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `darwin-rebuild switch` prints, abridged.
    const SAMPLE: &str = "\
building the system configuration...
these 3 derivations will be built:
  /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-foo-1.0.drv
  /nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bar-2.0.drv
  /nix/store/cccccccccccccccccccccccccccccccc-darwin-system-26.05.drv
these 2 paths will be fetched (12.3 MiB download, 40.1 MiB unpacked):
  /nix/store/dddddddddddddddddddddddddddddddd-baz-3.1
  /nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-qux-0.4
copying path '/nix/store/dddddddddddddddddddddddddddddddd-baz-3.1' from 'https://cache.nixos.org'...
copying path '/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-qux-0.4' from 'https://cache.nixos.org'...
building '/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-foo-1.0.drv'...
building '/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bar-2.0.drv'...
building '/nix/store/cccccccccccccccccccccccccccccccc-darwin-system-26.05.drv'...
setting up launchd services...
setting up /etc...
activating user profile...
Activating... done
";

    fn feed_all(text: &str) -> Progress {
        let mut progress = Progress::default();
        for line in text.lines() {
            progress.feed(line);
        }
        progress
    }

    #[test]
    fn parses_real_output() {
        assert_eq!(
            feed_all(SAMPLE),
            Progress {
                built: 3,
                to_build: 3,
                fetched: 2,
                to_fetch: 2,
                phase: Phase::Activating
            }
        );
    }

    #[test]
    fn singular_announcements() {
        let progress = feed_all(
            "this derivation will be built:\nthis path will be fetched (1 MiB download, 2 MiB unpacked):",
        );
        assert_eq!((progress.to_build, progress.to_fetch), (1, 1));
    }

    #[test]
    fn unrelated_lines_count_for_nothing() {
        let progress = feed_all(
            "building the system configuration...\ncopying 3 paths...\nthese are not the droids",
        );
        assert_eq!(progress, Progress::default());
    }

    fn phase_after(text: &str) -> Phase {
        feed_all(text).phase
    }

    #[test]
    fn phase_follows_the_output() {
        assert_eq!(phase_after(""), Phase::Evaluating);
        assert_eq!(
            phase_after("building the system configuration...\nevaluation warning: x"),
            Phase::Evaluating
        );
        assert_eq!(
            phase_after("these 3 derivations will be built:\n  /nix/store/a.drv"),
            Phase::Evaluating
        );
        assert_eq!(
            phase_after("these 2 paths will be fetched (1 MiB download, 2 MiB unpacked):"),
            Phase::Downloading
        );
        assert_eq!(
            phase_after("copying path '/nix/store/a-baz' from 'https://cache.nixos.org'..."),
            Phase::Downloading
        );
        assert_eq!(phase_after("copying 3 paths..."), Phase::Evaluating);
        assert_eq!(
            phase_after("building '/nix/store/a.drv'..."),
            Phase::Building
        );
    }

    #[test]
    fn building_wins_over_downloading() {
        assert_eq!(
            phase_after(
                "building '/nix/store/a.drv'...\ncopying path '/nix/store/b-baz' from 'https://cache.nixos.org'...\nthese 2 paths will be fetched (1 MiB download, 2 MiB unpacked):"
            ),
            Phase::Building
        );
        assert_eq!(
            phase_after(
                "copying path '/nix/store/b-baz' from 'ssh-ng://daniel@mini-builder'...\nbuilding '/nix/store/a.drv'..."
            ),
            Phase::Building
        );
    }

    #[test]
    fn activation_overrides_everything() {
        // Real darwin-rebuild output.
        let darwin = "building '/nix/store/h-darwin-system-26.11.4cff07d.drv'...\nsetting up /Applications/Nix Apps...\nsetting up pam...\nbuilding '/nix/store/x.drv'...\ncopying path '/nix/store/y' from 'z'...";
        assert_eq!(phase_after(darwin), Phase::Activating);
        // Real nixos-rebuild output (switch-to-configuration, then the activation script).
        let nixos = "building '/nix/store/h-nixos-system-harbor.drv'...\nactivating the configuration...\nsetting up /etc...";
        assert_eq!(phase_after(nixos), Phase::Activating);
        assert_eq!(
            phase_after("copying path '/nix/store/a' from 'b'...\nactivating the configuration..."),
            Phase::Activating
        );
        assert_eq!(
            phase_after("building '/nix/store/a.drv'...\nsetting up /etc..."),
            Phase::Activating
        );
    }

    #[test]
    fn setting_up_during_the_build_is_not_activation() {
        let early = "these 2 derivations will be built:\nbuilding '/nix/store/a.drv'...\nsetting up something...";
        assert_eq!(phase_after(early), Phase::Building);
        let done = "these 2 derivations will be built:\nbuilding '/nix/store/a.drv'...\nbuilding '/nix/store/b.drv'...\nsetting up /etc...";
        assert_eq!(phase_after(done), Phase::Activating);
        // Nothing to build: activation starts right away.
        assert_eq!(phase_after("setting up /etc..."), Phase::Activating);
    }

    #[test]
    fn old_files_without_a_phase_parse() {
        let json = r#"{"state":"running","pid":1,"started":2,"finished":null,"built":0,"to_build":0,"fetched":0,"to_fetch":0,"last_line":"x","error":null,"generation":null}"#;
        let status: Status = serde_json::from_str(json).unwrap();
        assert_eq!(status.phase, Phase::Evaluating);
        let with = json.replace("\"last_line\"", "\"phase\":\"building\",\"last_line\"");
        assert_eq!(
            serde_json::from_str::<Status>(&with).unwrap().phase,
            Phase::Building
        );
    }

    #[test]
    fn generation_from_link_name() {
        assert_eq!(parse_generation("system-279-link"), Some(279));
        assert_eq!(parse_generation("default"), None);
    }

    #[test]
    fn stale_running_state_counts_as_failed() {
        let dead = Status::running(i32::MAX as u32, 100, "x");
        let checked = check_alive(dead);
        assert_eq!(checked.state, State::Failed);
        assert_eq!(checked.error.as_deref(), Some(STOPPED));

        let alive = Status::running(std::process::id(), 100, "x");
        assert_eq!(check_alive(alive).state, State::Running);
    }

    #[test]
    fn summary_sentences() {
        let mut status = Status::running(1, 0, "");
        status.state = State::Ok;
        status.generation = Some(279);
        assert_eq!(summary(&status, "swift"), "swift is on generation 279");
        status.state = State::Failed;
        status.error = Some(AUTH_FAILED.into());
        assert_eq!(summary(&status, "swift"), AUTH_FAILED);
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn parses_arguments() {
        let parsed = parse(&strings(&[
            "--as-root",
            "--state-dir",
            "/home/d/.local/state/telmo",
            "--owner",
            "501:20",
            "--",
            "/bin/darwin-rebuild",
            "switch",
            "--flake",
            ".",
        ]))
        .unwrap();
        assert_eq!(parsed.mode, Mode::AsRoot);
        assert_eq!(parsed.dir, PathBuf::from("/home/d/.local/state/telmo"));
        assert_eq!(parsed.owner, Owner { uid: 501, gid: 20 });
        assert_eq!(parsed.argv[0], "/bin/darwin-rebuild");
        assert_eq!(parsed.argv.len(), 4);
        let child = parse(&strings(&[
            "--child",
            "--state-dir",
            "/s",
            "--owner",
            "1:2",
            "--",
            "x",
        ]))
        .unwrap();
        assert_eq!(child.mode, Mode::Child);
    }

    #[test]
    fn rejects_bad_arguments() {
        let bad = |args: &[&str]| parse(&strings(args)).is_err();
        assert!(bad(&[]));
        assert!(bad(&[
            "--child",
            "--state-dir",
            "/s",
            "--owner",
            "1:2",
            "--"
        ]));
        assert!(bad(&[
            "--child",
            "--state-dir",
            "/s",
            "--owner",
            "me",
            "--",
            "x"
        ]));
        assert!(bad(&[
            "--child",
            "--state-dir",
            "rel",
            "--owner",
            "1:2",
            "--",
            "x"
        ]));
        assert!(bad(&["--state-dir", "/s", "--owner", "1:2", "--", "x"]));
        assert!(bad(&["--child", "--owner", "1:2", "--", "x"]));
        assert!(bad(&["--nope", "--", "x"]));
    }

    /// Runs fake commands against a temp state directory (no root, so no chown).
    #[test]
    fn runner_writes_progress_and_result() {
        let dir = std::env::temp_dir().join(format!("telmo-rebuild-test-{}", std::process::id()));
        let job = |script: &str| Job {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            waiting: "starting…".into(),
            write_every: Duration::ZERO,
            dir: dir.clone(),
            owner: Owner { uid: 0, gid: 0 },
        };
        let saved = || -> Option<Status> {
            serde_json::from_slice(&std::fs::read(dir.join("rebuild.json")).ok()?).ok()
        };

        let mut seen = Vec::new();
        let ok = run(
            &job(
                "echo 'these 2 derivations will be built:'; echo \"building '/nix/store/a.drv'...\"; echo \"building '/nix/store/b.drv'...\"; echo done",
            ),
            &mut |s| seen.push(s.clone()),
        );
        assert_eq!(ok.state, State::Ok);
        assert_eq!((ok.built, ok.to_build), (2, 2));
        assert_eq!(seen[0].state, State::Running);
        assert_eq!(seen[0].last_line, "starting…");
        assert!(seen.iter().any(|s| s.built == 1 && s.to_build == 2));
        assert_eq!(seen.last().map(|s| s.state), Some(State::Ok));
        assert_eq!(saved().map(|s| s.state), Some(State::Ok));
        let log = std::fs::read_to_string(dir.join("rebuild.log")).unwrap_or_default();
        assert!(log.contains("building '/nix/store/b.drv'"));

        let failed = run(&job("echo 'error: boom'; exit 1"), &mut |_| {});
        assert_eq!(failed.state, State::Failed);
        assert_eq!(failed.error.as_deref(), Some(FAILED));
        let log = std::fs::read_to_string(dir.join("rebuild.log")).unwrap_or_default();
        assert!(
            !log.contains("building"),
            "the log is truncated at the start"
        );

        let missing = run(
            &Job {
                argv: vec!["/nonexistent/telmo-test".into()],
                ..job("")
            },
            &mut |_| {},
        );
        assert_eq!(missing.state, State::Failed);
        let log = std::fs::read_to_string(dir.join("rebuild.log")).unwrap_or_default();
        assert!(log.contains("Couldn't run /nonexistent/telmo-test"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
