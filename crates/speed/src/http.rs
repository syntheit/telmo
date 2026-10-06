//! Cloudflare endpoints and the latency probe.

use std::time::{Duration, Instant};

use reqwest::{Client, header::HeaderMap};

use crate::stats::{jitter, median};
use crate::{Fail, Update};

pub const BASE: &str = "https://speed.cloudflare.com";
const LATENCY_SAMPLES: usize = 20;

fn builder() -> reqwest::ClientBuilder {
    // HTTP/1.1 only: every client then owns real TCP connections, and
    // streams can't be multiplexed onto one.
    Client::builder()
        .user_agent(format!("telmo/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .http1_only()
}

/// For short requests: server info and latency probes.
pub fn client() -> Result<Client, Fail> {
    builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(reason)
}

/// For one transfer stream. No request timeout: the phase deadline ends it,
/// and a slow upload answers only after its whole body is sent.
pub fn stream_client() -> Result<Client, Fail> {
    builder().build().map_err(reason)
}

pub fn reason(e: reqwest::Error) -> Fail {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else if e.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) {
        "rate limited, try again in a while".into()
    } else if let Some(status) = e.status() {
        format!("server answered {status}")
    } else {
        "request failed".into()
    }
}

pub async fn server(client: &Client) -> Result<Update, Fail> {
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
pub async fn ping(client: &Client) -> Result<f64, Fail> {
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

/// Sequential pings on one warm connection: (median, jitter).
pub async fn idle_latency(client: &Client) -> Result<(f64, f64), Fail> {
    ping(client).await?; // warm-up: connection setup isn't latency
    let mut samples = Vec::new();
    for _ in 0..LATENCY_SAMPLES {
        samples.push(ping(client).await?);
    }
    Ok((median(&samples), jitter(&samples)))
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

#[cfg(test)]
mod tests {
    use super::*;

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
