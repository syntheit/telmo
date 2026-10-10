//! The hidden commands the watchers run. They never open a UI.
//!
//! `ingest --kind text|image|files --file <path> [--source <app>]` is run by
//! Telmo.app on macOS: the file holds UTF-8 text, a PNG, or newline-separated
//! absolute paths, and is deleted afterwards whatever happens.
//! `ingest-wayland` is run by `wl-paste --watch` on Linux.

use crate::config;
use crate::content::{Content, MAX_IMAGE, MAX_TEXT};
use crate::store::{Outcome, Store};
use crate::wayland;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

pub fn open_store() -> Result<Store, String> {
    let config = config::load()?;
    let dir =
        config::data_dir().ok_or("HOME isn't set, so there's nowhere to keep the history.")?;
    Ok(Store::new(dir, config))
}

pub fn report(result: Result<Outcome, String>) -> ExitCode {
    match result {
        Ok(Outcome::Stored) => ExitCode::SUCCESS,
        Ok(Outcome::Skipped(reason)) => {
            if !reason.is_empty() {
                eprintln!("telmo-clipboard: {reason}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("telmo-clipboard: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `ingest ...`: `args` are the words after the subcommand.
pub fn run(args: &[String]) -> ExitCode {
    let options = Options::parse(args);
    // The host leaves the file to us: remove it on every path out.
    let _cleanup = options.file.as_deref().map(Cleanup);
    let result = options
        .validate()
        .and_then(|(kind, file)| ingest_file(kind, file, options.source.trim()));
    report(result)
}

#[derive(Debug, Default)]
struct Options {
    kind: Option<String>,
    file: Option<PathBuf>,
    source: String,
    problem: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Self {
        let mut options = Options::default();
        let mut words = args.iter();
        while let Some(word) = words.next() {
            let slot = match word.as_str() {
                "--kind" | "--file" | "--source" => words.next(),
                other => {
                    options.problem = Some(format!("Unknown argument {other}."));
                    continue;
                }
            };
            let Some(value) = slot else {
                options.problem = Some(format!("{word} needs a value."));
                break;
            };
            match word.as_str() {
                "--kind" => options.kind = Some(value.clone()),
                "--file" => options.file = Some(PathBuf::from(value)),
                _ => options.source = value.clone(),
            }
        }
        options
    }

    fn validate(&self) -> Result<(&str, &Path), String> {
        if let Some(problem) = &self.problem {
            return Err(format!(
                "{problem} Usage: telmo-clipboard ingest --kind text|image|files --file <path> [--source <app>]"
            ));
        }
        let kind = self
            .kind
            .as_deref()
            .ok_or("Missing --kind text|image|files.")?;
        let file = self.file.as_deref().ok_or("Missing --file <path>.")?;
        Ok((kind, file))
    }
}

struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

fn ingest_file(kind: &str, file: &Path, source: &str) -> Result<Outcome, String> {
    let store = open_store()?;
    let content = match kind {
        "text" => match read_capped(file, MAX_TEXT)? {
            Some(bytes) => Content::Text(String::from_utf8_lossy(&bytes).into_owned()),
            None => return Ok(Outcome::Skipped("Skipped text over 1 MB.".into())),
        },
        "image" => match read_capped(file, MAX_IMAGE)? {
            Some(bytes) => Content::Image(bytes),
            None => return Ok(Outcome::Skipped("Skipped an image over 30 MB.".into())),
        },
        "files" => {
            let bytes = read_capped(file, MAX_TEXT)?.ok_or("The file list is too long.")?;
            Content::Files(
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .map(String::from)
                    .collect(),
            )
        }
        other => {
            return Err(format!(
                "Unknown kind \"{other}\". Use text, image or files."
            ));
        }
    };
    store.ingest(content, source, telmo_kit::time::unix_now())
}

/// The file's bytes, or `None` when it is bigger than `limit`.
fn read_capped(file: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let size = std::fs::metadata(file)
        .map_err(|e| format!("Can't read {}: {e}.", file.display()))?
        .len();
    if size > limit as u64 {
        return Ok(None);
    }
    std::fs::read(file)
        .map(Some)
        .map_err(|e| format!("Can't read {}: {e}.", file.display()))
}

/// `ingest-wayland`: what `wl-paste --watch` runs after every change.
pub fn run_wayland() -> ExitCode {
    report(wayland::ingest())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::tests::png;
    use crate::model::Kind;
    use std::sync::Mutex;

    /// The commands read XDG_* variables, which are process-wide.
    static ENV: Mutex<()> = Mutex::new(());

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Runs `body` with the data and config homes pointing at a temp dir.
    fn with_home(name: &str, body: impl FnOnce(&Path)) {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("telmo-ingest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: tests that touch the environment hold ENV.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home.join("data"));
            std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        }
        body(&home);
        let _ = std::fs::remove_dir_all(&home);
    }

    fn history() -> Vec<crate::model::Entry> {
        open_store().unwrap().entries().unwrap()
    }

    #[test]
    fn text_with_a_source_and_the_temp_file_is_removed() {
        with_home("text", |home| {
            let file = home.join("telmo-clip-1");
            std::fs::write(&file, "hello there").unwrap();
            let code = run(&args(&[
                "--kind",
                "text",
                "--file",
                file.to_str().unwrap(),
                "--source",
                "Ghostty",
            ]));
            assert_eq!(code, ExitCode::SUCCESS);
            assert!(!file.exists());
            let entries = history();
            assert_eq!(entries[0].preview, "hello there");
            assert_eq!(entries[0].source, "Ghostty");
        });
    }

    #[test]
    fn empty_source_means_unknown() {
        with_home("nosource", |home| {
            let file = home.join("t");
            std::fs::write(&file, "x y").unwrap();
            run(&args(&[
                "--kind",
                "text",
                "--file",
                file.to_str().unwrap(),
                "--source",
                "",
            ]));
            assert_eq!(history()[0].source, "");
        });
    }

    #[test]
    fn png_images() {
        with_home("image", |home| {
            let file = home.join("img");
            std::fs::write(&file, png(16, 9)).unwrap();
            let code = run(&args(&[
                "--kind",
                "image",
                "--file",
                file.to_str().unwrap(),
            ]));
            assert_eq!(code, ExitCode::SUCCESS);
            assert!(!file.exists());
            let entry = &history()[0];
            assert_eq!(
                (entry.kind, entry.width, entry.height),
                (Kind::Image, 16, 9)
            );
        });
    }

    #[test]
    fn file_lists() {
        with_home("files", |home| {
            let file = home.join("list");
            std::fs::write(&file, "/Users/me/Downloads/invoice.pdf\n/Users/me/a.txt\n").unwrap();
            let code = run(&args(&[
                "--kind",
                "files",
                "--file",
                file.to_str().unwrap(),
                "--source",
                "Finder",
            ]));
            assert_eq!(code, ExitCode::SUCCESS);
            assert!(!file.exists());
            let entry = &history()[0];
            assert_eq!(entry.kind, Kind::File);
            assert_eq!(entry.files.len(), 2);
            assert_eq!(entry.source, "Finder");
        });
    }

    #[test]
    fn failures_still_delete_the_file() {
        with_home("fail", |home| {
            let file = home.join("bad");
            std::fs::write(&file, "").unwrap();
            let code = run(&args(&["--kind", "text", "--file", file.to_str().unwrap()]));
            assert_eq!(code, ExitCode::FAILURE);
            assert!(!file.exists());

            std::fs::write(&file, "not a picture").unwrap();
            let code = run(&args(&[
                "--kind",
                "image",
                "--file",
                file.to_str().unwrap(),
            ]));
            assert_eq!(code, ExitCode::FAILURE);
            assert!(!file.exists());

            std::fs::write(&file, "x").unwrap();
            let code = run(&args(&[
                "--kind",
                "sound",
                "--file",
                file.to_str().unwrap(),
            ]));
            assert_eq!(code, ExitCode::FAILURE);
            assert!(!file.exists());

            std::fs::write(&file, "x").unwrap();
            let code = run(&args(&["--file", file.to_str().unwrap(), "--bogus"]));
            assert_eq!(code, ExitCode::FAILURE);
            assert!(!file.exists());
            assert!(history().is_empty());
        });
    }

    #[test]
    fn whitespace_and_oversized_are_skipped_quietly() {
        with_home("skip", |home| {
            let file = home.join("ws");
            std::fs::write(&file, "  \n ").unwrap();
            assert_eq!(
                run(&args(&["--kind", "text", "--file", file.to_str().unwrap()])),
                ExitCode::SUCCESS
            );
            std::fs::write(&file, "a".repeat(MAX_TEXT + 1)).unwrap();
            assert_eq!(
                run(&args(&["--kind", "text", "--file", file.to_str().unwrap()])),
                ExitCode::SUCCESS
            );
            assert!(history().is_empty());
            assert!(!file.exists());
        });
    }

    #[test]
    fn a_missing_file_is_an_error() {
        with_home("missing", |home| {
            let file = home.join("nope");
            assert_eq!(
                run(&args(&["--kind", "text", "--file", file.to_str().unwrap()])),
                ExitCode::FAILURE
            );
        });
    }
}
