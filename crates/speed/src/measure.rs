//! Time-based throughput: parallel streams on separate connections feed one
//! byte counter, which is sampled every 100 ms.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{StreamExt, stream};
use reqwest::{Body, Client};
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep};

use crate::http::{BASE, client, ping, reason, stream_client};
use crate::stats::{self, BASE_SECS, MAX_SECS, Sample};
use crate::{Fail, Phase, Update};

const DOWN_STREAMS: usize = 6;
const UP_STREAMS: usize = 4;
/// Cloudflare answers 403 from 100_000_000 bytes up.
const DOWN_BYTES: u64 = 99_000_000;
const UP_BYTES: usize = 25_000_000;
const UP_CHUNK: usize = 64 * 1024;
const SAMPLE_EVERY: Duration = Duration::from_millis(100);
const LOADED_PING_EVERY: Duration = Duration::from_millis(250);
/// No bytes by then means the test can't run.
const FIRST_BYTES_SECS: f64 = 4.0;

type Counter = Arc<AtomicU64>;

/// One download or upload phase: progress updates, then the final `Result`.
pub async fn transfer(phase: Phase, on_update: &mut impl FnMut(Update)) -> Result<(), Fail> {
    let counter = Counter::default();
    let streams = if phase == Phase::Upload {
        UP_STREAMS
    } else {
        DOWN_STREAMS
    };
    let mut set = JoinSet::new();
    for _ in 0..streams {
        set.spawn(run_stream(stream_client()?, phase, counter.clone()));
    }

    let loaded = Arc::new(Mutex::new(Vec::new()));
    let prober = tokio::spawn(probe_loaded(client()?, loaded.clone()));

    let sampled = sample(phase, &counter, &mut set, on_update).await;
    set.abort_all(); // partial bodies count
    prober.abort();
    let samples = sampled?;

    let mbps = stats::final_mbps(&samples);
    on_update(Update::Progress {
        phase,
        mbps,
        fraction: 1.0,
    });
    let loaded = loaded.lock().map(|l| l.clone()).unwrap_or_default();
    on_update(Update::Result {
        phase,
        mbps,
        loaded_ms: (!loaded.is_empty()).then(|| stats::median(&loaded)),
    });
    Ok(())
}

/// Samples the counter until the phase is over, reporting live speed.
async fn sample(
    phase: Phase,
    counter: &AtomicU64,
    streams: &mut JoinSet<Fail>,
    on_update: &mut impl FnMut(Update),
) -> Result<Vec<Sample>, Fail> {
    let start = Instant::now();
    let mut samples = vec![(0.0, 0)];
    let mut end = BASE_SECS;
    let mut tick = interval(SAMPLE_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_error = None;

    loop {
        tokio::select! {
            _ = tick.tick() => {}
            Some(done) = streams.join_next() => {
                last_error = Some(done.unwrap_or_else(|_| "request failed".into()));
                if streams.is_empty() {
                    return Err(last_error.unwrap_or_default());
                }
                continue;
            }
        }
        let secs = start.elapsed().as_secs_f64();
        let bytes = counter.load(Ordering::Relaxed);
        samples.push((secs, bytes));

        if bytes == 0 && secs >= FIRST_BYTES_SECS {
            return Err(last_error.unwrap_or_else(|| "no data received".into()));
        }
        on_update(Update::Progress {
            phase,
            mbps: stats::live_mbps(&samples),
            fraction: (secs / BASE_SECS).min(1.0) as f32,
        });
        if secs >= end {
            if end == BASE_SECS && stats::needs_extension(&samples) {
                end = MAX_SECS;
            } else {
                return Ok(samples);
            }
        }
    }
}

/// Loops one request kind on one connection until aborted. Returns why it stopped.
async fn run_stream(client: Client, phase: Phase, counter: Counter) -> Fail {
    let upload = Bytes::from(noise(UP_CHUNK));
    loop {
        let result = match phase {
            Phase::Upload => upload_once(&client, &upload, &counter).await,
            _ => download_once(&client, &counter).await,
        };
        if let Err(e) = result {
            return e;
        }
    }
}

async fn download_once(client: &Client, counter: &AtomicU64) -> Result<(), Fail> {
    let mut resp = client
        .get(format!("{BASE}/__down?bytes={DOWN_BYTES}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(reason)?;
    while let Some(chunk) = resp.chunk().await.map_err(reason)? {
        counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// Counts bytes as hyper pulls them from the body, so it runs ahead of the
/// wire by about the socket buffer; the warm-up window hides that.
async fn upload_once(client: &Client, chunk: &Bytes, counter: &Counter) -> Result<(), Fail> {
    let (chunk, counter) = (chunk.clone(), counter.clone());
    let chunks = UP_BYTES / UP_CHUNK;
    let body = stream::iter(0..chunks).map(move |_| {
        counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        Ok::<_, std::convert::Infallible>(chunk.clone())
    });
    client
        .post(format!("{BASE}/__up"))
        .body(Body::wrap_stream(body))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(reason)?;
    Ok(())
}

/// Incompressible-looking filler (xorshift), made once per stream.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Latency under load, on its own connection. The first ping pays for
/// connection setup and is dropped.
async fn probe_loaded(client: Client, samples: Arc<Mutex<Vec<f64>>>) {
    let _ = ping(&client).await;
    loop {
        sleep(LOADED_PING_EVERY).await;
        if let (Ok(ms), Ok(mut all)) = (ping(&client).await, samples.lock()) {
            all.push(ms);
        }
    }
}
