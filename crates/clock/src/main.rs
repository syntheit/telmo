mod app;
mod cli;
mod config;
mod fmt;
mod model;
mod notify;
mod parse;
mod sched;
mod store;
mod ui;

use jiff::tz::TimeZone;
use std::process::ExitCode;
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if cli::is_command(&args) {
        return match cli::run(&args) {
            Ok(out) => {
                if !out.is_empty() {
                    println!("{out}");
                }
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("telmo-clock: {message}");
                ExitCode::FAILURE
            }
        };
    }
    let opts = telmo_kit::cli::args();
    if opts.status {
        let state = store::load();
        println!("{}", cli::timer_list(&state, cli::now_ms()));
        return ExitCode::SUCCESS;
    }
    let (config, warning) = match config::load() {
        Ok(config) => (config, None),
        Err(message) => (config::Config::default(), Some(message)),
    };
    let state = if opts.mock {
        app::mock_state(cli::now_ms())
    } else {
        store::load()
    };
    let mut app = app::App::new(state, config.places(), TimeZone::system(), !opts.mock);
    if let Some(message) = warning {
        app.warn(message);
    }
    let (_keep_open, events) = unbounded_channel::<()>();
    match telmo_kit::run(app, events).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("telmo-clock: Could not use the terminal: {e}");
            ExitCode::FAILURE
        }
    }
}
