//! The background rebuild. `telmo-system rebuild-run` is a detached process
//! that runs the configured command with elevation, follows its output and
//! keeps `rebuild.json` and `rebuild.log` up to date. The popup only reads them.

use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STATE: &str = "rebuild";
const PROFILE: &str = "/nix/var/nix/profiles/system";
const WRITE_EVERY: Duration = Duration::from_millis(500);

pub const NOT_CONFIGURED: &str = "Set programs.telmo.system.rebuild to rebuild from here.";
pub const ALREADY_RUNNING: &str = "A rebuild is already running.";
const TOUCH_ID: &str = "Touch ID didn't confirm the rebuild. Press u to try again.";
const DISMISSED: &str = "The password dialog was dismissed.";
const FAILED: &str = "The rebuild failed. Press L to see the log.";
const STOPPED: &str = "The rebuild stopped unexpectedly.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Ok,
    Failed,
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

/// What the waiting-for-authentication line says on this OS.
pub fn waiting_text() -> &'static str {
    if cfg!(target_os = "macos") {
        "waiting for Touch ID…"
    } else {
        "waiting for the password…"
    }
}

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

/// Starts the detached runner. The runner writes the state file itself.
pub fn start() -> Result<(), String> {
    if read().is_some_and(|s| s.state == State::Running) {
        return Err(ALREADY_RUNNING.into());
    }
    let exe = std::env::current_exe()
        .map_err(|_| "Couldn't find the telmo-system program to run the rebuild.".to_string())?;
    let mut command = Command::new(exe);
    command
        .arg("rebuild-run")
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
        .map_err(|e| format!("Couldn't start the rebuild ({e}). Press u to try again."))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Counts of builds and fetches, read from Nix's output.
#[derive(Debug, Default, PartialEq)]
pub struct Progress {
    pub built: u32,
    pub to_build: u32,
    pub fetched: u32,
    pub to_fetch: u32,
}

impl Progress {
    pub fn feed(&mut self, line: &str) {
        let line = line.trim();
        if let Some(n) = announced(line, "derivation", "will be built") {
            self.to_build = n;
        } else if let Some(n) = announced(line, "path", "will be fetched") {
            self.to_fetch = n;
        } else if line.starts_with("building '") {
            self.built += 1;
        } else if line.starts_with("copying path '") {
            self.fetched += 1;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Sudo,
    Pkexec,
    #[cfg(test)]
    None,
}

/// The sentence for a command that ended badly.
fn failure_sentence(elevation: Elevation, code: Option<i32>, output: &str, silent: bool) -> String {
    let output = output.to_lowercase();
    let refused = [
        "no password was provided",
        "a password is required",
        "incorrect password",
    ]
    .iter()
    .any(|phrase| output.contains(phrase));
    match elevation {
        Elevation::Sudo if refused || silent => TOUCH_ID.into(),
        Elevation::Pkexec if matches!(code, Some(126 | 127)) => DISMISSED.into(),
        _ => FAILED.into(),
    }
}

/// One rebuild to run.
pub struct Job {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub elevation: Elevation,
    pub waiting: String,
    /// Smallest gap between two state-file writes while output streams in.
    pub write_every: Duration,
}

impl Job {
    /// The configured command wrapped in the OS's elevation.
    pub fn elevated(command: &[String]) -> Result<Self, String> {
        let program = command
            .first()
            .ok_or("The rebuild command is empty. Check programs.telmo.system.rebuild.")?;
        let (argv, env, elevation) = if cfg!(target_os = "macos") {
            // Touch ID or nothing: /usr/bin/false makes sudo give up instead of asking.
            let mut argv = vec!["/usr/bin/sudo".to_string(), "-A".into()];
            argv.extend(command.iter().cloned());
            let env = vec![("SUDO_ASKPASS".to_string(), "/usr/bin/false".to_string())];
            (argv, env, Elevation::Sudo)
        } else {
            // pkexec sanitizes PATH, so it needs absolute paths.
            let pkexec =
                find_program("pkexec").ok_or("Couldn't find pkexec. Is polkit installed?")?;
            let program = find_program(program).ok_or_else(|| {
                format!("Couldn't find `{program}`. Check programs.telmo.system.rebuild.")
            })?;
            let mut argv = vec![pkexec, program];
            argv.extend(command.iter().skip(1).cloned());
            (argv, Vec::new(), Elevation::Pkexec)
        };
        Ok(Self {
            argv,
            env,
            elevation,
            waiting: waiting_text().into(),
            write_every: WRITE_EVERY,
        })
    }
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

fn save(status: &Status, observe: &mut dyn FnMut(&Status)) {
    // Nobody can be told about a failed write here; the log still has the output.
    let _ = telmo_kit::state::save(STATE, status);
    observe(status);
}

/// Runs the job to its end, keeping the state file and log current.
/// `observe` sees every status that is written.
pub fn run(job: &Job, observe: &mut dyn FnMut(&Status)) -> Status {
    let mut status = Status::running(std::process::id(), unix_now(), &job.waiting);
    save(&status, observe);
    let mut log = log_path().and_then(|path| {
        std::fs::create_dir_all(path.parent()?).ok()?;
        File::create(path).ok()
    });
    let (code, output, lines) = match follow(job, &mut status, &mut log, observe) {
        Ok(result) => result,
        Err(message) => {
            if let Some(log) = log.as_mut() {
                let _ = writeln!(log, "{message}");
            }
            (None, message, 0)
        }
    };
    let now = unix_now();
    status.finished = Some(now);
    if code == Some(0) {
        status.state = State::Ok;
        status.generation = current_generation();
    } else {
        let tail = output.lines().rev().take(20).collect::<Vec<_>>().join("\n");
        status.state = State::Failed;
        status.error = Some(failure_sentence(job.elevation, code, &tail, lines == 0));
    }
    save(&status, observe);
    status
}

/// Starts the command and follows its output. Returns the exit code, the
/// output text and how many lines there were.
fn follow(
    job: &Job,
    status: &mut Status,
    log: &mut Option<File>,
    observe: &mut dyn FnMut(&Status),
) -> Result<(Option<i32>, String, usize), String> {
    let (reader, writer) =
        std::io::pipe().map_err(|e| format!("Couldn't read the output ({e})."))?;
    let stderr = writer
        .try_clone()
        .map_err(|e| format!("Couldn't read the output ({e})."))?;
    let (program, args) = job.argv.split_first().ok_or("Empty command.")?;
    let mut child = Command::new(program)
        .args(args)
        .envs(job.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Couldn't run {program}: {e}"))?;

    let mut progress = Progress::default();
    let mut output = String::new();
    let mut lines = 0;
    let mut last_write = Instant::now();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        lines += 1;
        if let Some(log) = log.as_mut() {
            let _ = writeln!(log, "{line}");
        }
        output.push_str(&line);
        output.push('\n');
        progress.feed(&line);
        if !line.trim().is_empty() {
            status.last_line = line.trim().to_string();
        }
        status.built = progress.built;
        status.to_build = progress.to_build;
        status.fetched = progress.fetched;
        status.to_fetch = progress.to_fetch;
        if last_write.elapsed() >= job.write_every {
            last_write = Instant::now();
            save(status, observe);
        }
    }
    let code = child.wait().map_err(|e| format!("Lost the rebuild: {e}"))?;
    Ok((code.code(), output, lines))
}

/// The sentence the notification and the popup's footer show at the end.
pub fn summary(status: &Status, host: &str) -> String {
    match (status.state, status.generation) {
        (State::Ok, Some(generation)) => format!("{host} is on generation {generation}"),
        (State::Ok, None) => format!("{host} is rebuilt"),
        _ => status.error.clone().unwrap_or_else(|| FAILED.into()),
    }
}

fn notify(text: &str) {
    let mut command = if cfg!(target_os = "macos") {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        let mut command = Command::new("osascript");
        command.arg("-e").arg(format!(
            "display notification \"{escaped}\" with title \"telmo\""
        ));
        command
    } else {
        let mut command = Command::new("notify-send");
        command.arg("telmo").arg(text);
        command
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Entry point of `telmo-system rebuild-run`.
pub fn main(configured: Option<Vec<String>>, host: String) -> ExitCode {
    let job = configured
        .ok_or_else(|| NOT_CONFIGURED.to_string())
        .and_then(|command| Job::elevated(&command));
    let status = match job {
        Ok(job) => run(&job, &mut |_| {}),
        Err(sentence) => {
            let now = unix_now();
            let status = Status::running(std::process::id(), now, "").failed(&sentence, now);
            save(&status, &mut |_| {});
            status
        }
    };
    notify(&summary(&status, &host));
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
                to_fetch: 2
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

    #[test]
    fn generation_from_link_name() {
        assert_eq!(parse_generation("system-279-link"), Some(279));
        assert_eq!(parse_generation("default"), None);
    }

    #[test]
    fn classifies_failures() {
        use Elevation::*;
        let sudo = |output, silent| failure_sentence(Sudo, Some(1), output, silent);
        assert_eq!(sudo("sudo: no password was provided", false), TOUCH_ID);
        assert_eq!(sudo("sudo: 1 incorrect password attempt", false), TOUCH_ID);
        assert_eq!(sudo("", true), TOUCH_ID);
        assert_eq!(sudo("error: build of foo failed", false), FAILED);
        assert_eq!(failure_sentence(Pkexec, Some(126), "", true), DISMISSED);
        assert_eq!(failure_sentence(Pkexec, Some(127), "", false), DISMISSED);
        assert_eq!(failure_sentence(Pkexec, Some(1), "oops", false), FAILED);
        assert_eq!(failure_sentence(None, Some(1), "", true), FAILED);
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
        status.error = Some(DISMISSED.into());
        assert_eq!(summary(&status, "swift"), DISMISSED);
    }

    /// Runs fake commands against a temp state directory. One test, because
    /// the state directory comes from a process-wide environment variable.
    #[test]
    fn runner_writes_progress_and_result() {
        let dir = std::env::temp_dir().join(format!("telmo-rebuild-test-{}", std::process::id()));
        // SAFETY: this is the only test that touches the environment.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };

        let job = |script: &str| Job {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: Vec::new(),
            elevation: Elevation::None,
            waiting: "waiting for Touch ID…".into(),
            write_every: Duration::ZERO,
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
        assert_eq!(seen[0].last_line, "waiting for Touch ID…");
        assert!(seen.iter().any(|s| s.built == 1 && s.to_build == 2));
        assert_eq!(seen.last().map(|s| s.state), Some(State::Ok));
        assert_eq!(read().map(|s| s.state), Some(State::Ok));
        let log = std::fs::read_to_string(dir.join("telmo/rebuild.log")).unwrap_or_default();
        assert!(log.contains("building '/nix/store/b.drv'"));

        let failed = run(&job("echo 'error: boom'; exit 1"), &mut |_| {});
        assert_eq!(failed.state, State::Failed);
        assert_eq!(failed.error.as_deref(), Some(FAILED));
        let log = std::fs::read_to_string(dir.join("telmo/rebuild.log")).unwrap_or_default();
        assert!(
            !log.contains("building"),
            "the log is truncated at the start"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
