//! Arguments every module accepts.

use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct Args {
    /// Use fake data instead of the real system.
    pub mock: bool,
    /// Print one snapshot as JSON and exit, instead of opening the UI.
    pub status: bool,
}

pub fn args() -> Args {
    let mut args = Args {
        mock: false,
        status: false,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--mock" => args.mock = true,
            "status" => args.status = true,
            "-h" | "--help" => {
                let name = std::env::args().next().unwrap_or_default();
                println!("usage: {name} [--mock] [status]");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    args
}

/// For `status`: the first event `pick` turns into a snapshot, or `None` when
/// the backend says nothing useful within five seconds or stops.
pub async fn first_snapshot<E, S>(
    events: &mut UnboundedReceiver<E>,
    pick: impl Fn(E) -> Option<S>,
) -> Option<S> {
    let first = async {
        while let Some(event) = events.recv().await {
            if let Some(snapshot) = pick(event) {
                return Some(snapshot);
            }
        }
        None
    };
    tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .ok()
        .flatten()
}
