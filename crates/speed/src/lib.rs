//! Speedtest against speed.cloudflare.com. Same code on every OS.
//!
//! `run` reports progress through a callback so the caller can forward it
//! into its own event channel.

mod http;
mod measure;
mod stats;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Latency,
    Download,
    Upload,
}

#[derive(Debug, Clone)]
pub enum Update {
    /// Which Cloudflare location serves the test.
    Server {
        colo: String,
        city: Option<String>,
    },
    /// Idle latency (median) and jitter.
    Latency {
        idle_ms: f64,
        jitter_ms: f64,
    },
    /// Live throughput during a phase. `fraction` is phase progress 0..=1.
    Progress {
        phase: Phase,
        mbps: f64,
        fraction: f32,
    },
    /// Final number for a transfer phase, with latency measured under load.
    Result {
        phase: Phase,
        mbps: f64,
        loaded_ms: Option<f64>,
    },
    Finished,
    Failed(String),
}

/// One finished run, kept in a small local history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Unix seconds.
    pub at: u64,
    pub network: Option<String>,
    pub down_mbps: f64,
    pub up_mbps: f64,
    pub latency_ms: f64,
}

const HISTORY_LEN: usize = 5;

type Fail = String;

/// Run a full test. Never panics; failures arrive as `Update::Failed`.
pub async fn run(mut on_update: impl FnMut(Update) + Send) {
    if let Err(reason) = run_inner(&mut on_update).await {
        on_update(Update::Failed(format!(
            "Can't reach speed.cloudflare.com: {reason}."
        )));
    }
}

async fn run_inner(on_update: &mut impl FnMut(Update)) -> Result<(), Fail> {
    let client = http::client()?;
    on_update(http::server(&client).await?);

    let (idle_ms, jitter_ms) = http::idle_latency(&client).await?;
    on_update(Update::Latency { idle_ms, jitter_ms });

    for phase in [Phase::Download, Phase::Upload] {
        measure::transfer(phase, on_update).await?;
    }
    on_update(Update::Finished);
    Ok(())
}

fn history_path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(state.join("telmo/speed.json"))
}

/// Most recent first.
pub fn history() -> Vec<Record> {
    history_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn remember(record: Record) {
    let Some(path) = history_path() else { return };
    let mut records = history();
    records.insert(0, record);
    records.truncate(HISTORY_LEN);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&records) {
        let _ = std::fs::write(path, json);
    }
}
