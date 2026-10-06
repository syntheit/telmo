//! Speedtest against speed.cloudflare.com. Same code on every OS.
//!
//! `run` reports progress through a callback so the caller can forward it
//! into its own event channel.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use futures_util::{StreamExt, stream};
use reqwest::{Body, Client, header::HeaderMap};
use serde::{Deserialize, Serialize};
use tokio::time::{interval, sleep};

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

const BASE: &str = "https://speed.cloudflare.com";
const SIZES: [u64; 5] = [100_000, 1_000_000, 10_000_000, 25_000_000, 100_000_000];
/// Progress fraction is measured against the first four sizes.
const PLANNED_STEPS: f32 = 4.0;
const PARALLEL: usize = 4;
const PARALLEL_FROM: u64 = 10_000_000;
const LATENCY_SAMPLES: usize = 20;
const HISTORY_LEN: usize = 5;
/// Skip a step that would take longer than this at the previous step's speed.
const MAX_STEP_SECS: f64 = 3.0;
/// No phase runs longer than this; a step cut short still yields a sample.
const PHASE_SECS: u64 = 10;

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
    let client = Client::builder()
        .user_agent(format!("telmo/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(reason)?;

    on_update(server(&client).await?);

    let (idle_ms, jitter_ms) = idle_latency(&client).await?;
    on_update(Update::Latency { idle_ms, jitter_ms });

    for phase in [Phase::Download, Phase::Upload] {
        transfer(&client, phase, on_update).await?;
    }
    on_update(Update::Finished);
    Ok(())
}

fn reason(e: reqwest::Error) -> Fail {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else if let Some(status) = e.status() {
        format!("server answered {status}")
    } else {
        "request failed".into()
    }
}

async fn server(client: &Client) -> Result<Update, Fail> {
    let meta: serde_json::Value = client
        .get(format!("{BASE}/meta"))
        .header("Referer", "https://speed.cloudflare.com/")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(reason)?
        .json()
        .await
        .map_err(reason)?;
    let colo = &meta["colo"];
    // `colo` is an object with iata + city; older replies had just the code.
    let code = colo["iata"].as_str().or(colo.as_str()).unwrap_or("?");
    let city = colo["city"].as_str().map(str::to_owned);
    Ok(Update::Server {
        colo: code.to_owned(),
        city,
    })
}

/// Time to response headers minus the server's own processing time, in ms.
async fn ping(client: &Client) -> Result<f64, Fail> {
    let start = Instant::now();
    let resp = client
        .get(format!("{BASE}/__down?bytes=0"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(reason)?;
    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    let server = server_ms(resp.headers()).unwrap_or(0.0);
    Ok((total_ms - server).max(0.1))
}

async fn idle_latency(client: &Client) -> Result<(f64, f64), Fail> {
    ping(client).await?; // warm-up: connection setup isn't latency
    let mut samples = Vec::new();
    for _ in 0..LATENCY_SAMPLES {
        samples.push(ping(client).await?);
    }
    Ok((median(&samples), jitter(&samples)))
}

async fn transfer(
    client: &Client,
    phase: Phase,
    on_update: &mut impl FnMut(Update),
) -> Result<(), Fail> {
    let counter = Arc::new(AtomicU64::new(0));
    let mut throughputs = Vec::new();
    let mut loaded = Vec::new();
    let mut last_mbps: Option<f64> = None;
    let deadline = Instant::now() + Duration::from_secs(PHASE_SECS);

    for (done, size) in SIZES.into_iter().enumerate() {
        let connections = if size >= PARALLEL_FROM { PARALLEL } else { 1 };
        let total = size * connections as u64;
        if let Some(mbps) = last_mbps.filter(|_| size >= PARALLEL_FROM)
            && total as f64 * 8.0 / (mbps * 1e6) > MAX_STEP_SECS
        {
            break;
        }

        let base = counter.load(Ordering::Relaxed);
        let requests = (0..connections).map(|_| request(client, phase, size, &counter));
        let step = async {
            tokio::select! {
                results = join_all(requests) => {
                    let secs = results.into_iter().collect::<Result<Vec<f64>, Fail>>()?;
                    Ok::<f64, Fail>(secs.into_iter().fold(0.0, f64::max))
                }
                _ = report(on_update, phase, &counter, base, total, done as f32) => unreachable!(),
                _ = probe_loaded(client, &mut loaded) => unreachable!(),
            }
        };
        let started = Instant::now();
        let slowest: f64 = match tokio::time::timeout_at(deadline.into(), step).await {
            Ok(result) => result?,
            Err(_) => {
                // Out of time mid-step: judge by what actually moved.
                let moved = counter.load(Ordering::Relaxed) - base;
                let secs = started.elapsed().as_secs_f64();
                throughputs.push(moved as f64 * 8.0 / secs / 1e6);
                break;
            }
        };

        if slowest >= 0.010 {
            let mbps = total as f64 * 8.0 / slowest / 1e6;
            throughputs.push(mbps);
            last_mbps = Some(mbps);
        }
        if slowest > 1.0 || Instant::now() >= deadline {
            break;
        }
    }

    let mbps = percentile(&throughputs, 0.9);
    on_update(Update::Progress {
        phase,
        mbps,
        fraction: 1.0,
    });
    on_update(Update::Result {
        phase,
        mbps,
        loaded_ms: (!loaded.is_empty()).then(|| median(&loaded)),
    });
    Ok(())
}

/// One download or upload. Returns seconds taken. Server-Timing isn't
/// subtracted: for uploads the worker time includes receiving the body.
async fn request(
    client: &Client,
    phase: Phase,
    size: u64,
    counter: &Arc<AtomicU64>,
) -> Result<f64, Fail> {
    let start = Instant::now();
    let mut resp = match phase {
        Phase::Upload => {
            let body = Body::wrap_stream(upload_body(size, counter.clone()));
            client.post(format!("{BASE}/__up")).body(body)
        }
        _ => client.get(format!("{BASE}/__down?bytes={size}")),
    }
    .send()
    .await
    .and_then(|r| r.error_for_status())
    .map_err(reason)?;

    if phase == Phase::Download {
        while let Some(chunk) = resp.chunk().await.map_err(reason)? {
            counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
    }
    Ok(start.elapsed().as_secs_f64().max(0.001))
}

/// Zeros in 64 kB chunks, counted as hyper pulls them.
fn upload_body(
    size: u64,
    counter: Arc<AtomicU64>,
) -> impl futures_util::Stream<Item = Result<Vec<u8>, Infallible>> {
    const CHUNK: u64 = 64 * 1024;
    let chunks = size.div_ceil(CHUNK);
    stream::iter(0..chunks).map(move |i| {
        let len = CHUNK.min(size - i * CHUNK);
        counter.fetch_add(len, Ordering::Relaxed);
        Ok(vec![0u8; len as usize])
    })
}

/// Emits `Progress` every 100 ms, with speed over the last 500 ms.
async fn report(
    on_update: &mut impl FnMut(Update),
    phase: Phase,
    counter: &AtomicU64,
    base: u64,
    total: u64,
    done: f32,
) {
    let mut window = VecDeque::from([(Instant::now(), counter.load(Ordering::Relaxed))]);
    let mut tick = interval(Duration::from_millis(100));
    tick.tick().await;
    loop {
        tick.tick().await;
        let (now, bytes) = (Instant::now(), counter.load(Ordering::Relaxed));
        window.push_back((now, bytes));
        while window.len() > 2 && now - window[1].0 >= Duration::from_millis(500) {
            window.pop_front();
        }
        let (t0, b0) = window[0];
        let mbps = (bytes - b0) as f64 * 8.0 / (now - t0).as_secs_f64() / 1e6;
        let step = ((bytes - base) as f32 / total as f32).min(1.0);
        on_update(Update::Progress {
            phase,
            mbps,
            fraction: ((done + step) / PLANNED_STEPS).min(1.0),
        });
    }
}

/// Latency under load: an occasional empty request while data flows.
async fn probe_loaded(client: &Client, samples: &mut Vec<f64>) {
    loop {
        sleep(Duration::from_millis(120)).await;
        if let Ok(ms) = ping(client).await {
            samples.push(ms);
        }
    }
}

/// Server processing time from `Server-Timing` headers, in ms.
fn server_ms(headers: &HeaderMap) -> Option<f64> {
    let values = headers
        .get_all("server-timing")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    parse_server_timing(&values)
}

fn parse_server_timing(header: &str) -> Option<f64> {
    let dur = |name: &str| {
        header.split(',').find_map(|metric| {
            let mut parts = metric.split(';').map(str::trim);
            if parts.next()? != name {
                return None;
            }
            parts.find_map(|p| p.strip_prefix("dur=")?.parse().ok())
        })
    };
    dur("cfRequestDuration").or_else(|| dur("cfSpeedWorker"))
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Mean absolute difference of consecutive samples.
fn jitter(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let sum: f64 = values.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    sum / (values.len() - 1) as f64
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd_even_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn jitter_is_mean_abs_diff() {
        assert_eq!(jitter(&[10.0, 12.0, 11.0, 15.0]), (2.0 + 1.0 + 4.0) / 3.0);
        assert_eq!(jitter(&[5.0]), 0.0);
    }

    #[test]
    fn p90_picks_high_sample() {
        let v: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.9), 9.0);
        assert_eq!(percentile(&[7.0], 0.9), 7.0);
        assert_eq!(percentile(&[], 0.9), 0.0);
    }

    #[test]
    fn server_timing_parsing() {
        assert_eq!(
            parse_server_timing("cfSpeedEdge;dur=9, cfSpeedWorker;dur=24"),
            Some(24.0)
        );
        assert_eq!(
            parse_server_timing("cfSpeedWorker;dur=26,cfRequestDuration;dur=12.5"),
            Some(12.5)
        );
        assert_eq!(parse_server_timing("cfL4;desc=\"?rtt=1\""), None);
        assert_eq!(parse_server_timing(""), None);
    }
}
