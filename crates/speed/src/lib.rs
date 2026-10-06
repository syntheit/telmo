//! Speedtest against speed.cloudflare.com. Same code on every OS.
//!
//! `run` reports progress through a callback so the caller can forward it
//! into its own event channel.

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

/// Run a full test. Never panics; failures arrive as `Update::Failed`.
pub async fn run(mut on_update: impl FnMut(Update) + Send) {
    on_update(Update::Failed("speedtest not implemented yet".into()));
}

/// Most recent first.
pub fn history() -> Vec<Record> {
    Vec::new()
}

pub fn remember(_record: Record) {}
