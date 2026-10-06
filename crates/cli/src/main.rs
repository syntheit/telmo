//! `telmo`: dispatcher for the popup modules and the macOS host app.

mod popup;

use std::env;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

const MODULES: [&str; 3] = ["net", "bt", "sound"];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("list") => list(),
        Some("popup") => match args.get(1) {
            Some(module) => popup::toggle(module),
            None => Err("usage: telmo popup <module>".into()),
        },
        Some("host") if args.len() > 1 => {
            popup::host_command(&args[1..].join(" ")).map(|reply| println!("{reply}"))
        }
        Some("host") => Err("usage: telmo host <command>".into()),
        Some(module) => run(module, &args[1..]),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("telmo: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Path of `telmo-<module>`: next to this executable first, then on PATH.
pub fn find_module(module: &str) -> Option<PathBuf> {
    let name = format!("telmo-{module}");
    let own_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    let path_dirs = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    own_dir
        .into_iter()
        .chain(path_dirs)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
}

fn list() -> Result<(), String> {
    let found: Vec<_> = MODULES
        .iter()
        .filter(|m| find_module(m).is_some())
        .collect();
    if found.is_empty() {
        return Err("no modules installed (looked for telmo-net, telmo-bt, telmo-sound next to telmo and on PATH)".into());
    }
    for module in found {
        println!("{module}");
    }
    Ok(())
}

fn run(module: &str, args: &[String]) -> Result<(), String> {
    let path = find_module(module).ok_or_else(|| {
        format!("unknown module '{module}'; run `telmo list` to see what is installed")
    })?;
    let error = Command::new(&path).args(args).exec();
    Err(format!("cannot run {}: {error}", path.display()))
}
