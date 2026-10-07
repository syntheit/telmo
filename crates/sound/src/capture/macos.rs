//! Listens to everything playing on the Mac through the Telmo host app. The
//! "System Audio Recording" grant belongs to the app, not to this process, so
//! the app runs the Core Audio tap and streams mono samples over its socket:
//! `audio-stream`, a header line `ok rate=<hz>`, then little-endian f32.
//! Closing the connection stops the tap.
//!
//! Without the permission the host answers `error denied`, or the tap delivers
//! silence. Both end in `Event::VisualizerBlocked`.

use super::{FRAME, stopped};
use crate::backend::{Event, Tx};
use crate::spectrum::Analyzer;
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

/// Exact silence this long while something plays means we are not allowed in.
const SILENT_FOR: Duration = Duration::from_secs(4);
/// The host has to create the tap, and the first time macOS asks the user.
const HEADER_WAIT: Duration = Duration::from_secs(30);

pub fn run(stop: &AtomicBool, events: &Tx) {
    match stream(stop, events) {
        Ok(()) => {}
        Err(Failure::Denied) => {
            let _ = events.send(Event::VisualizerBlocked);
        }
        Err(Failure::Other(message)) => {
            let _ = events.send(Event::Failed(message));
        }
    }
}

enum Failure {
    Denied,
    Other(String),
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Other(format!(
            "Couldn't reach the Telmo host, so there is no visualizer: {e}."
        ))
    }
}

fn socket_path() -> Result<PathBuf, Failure> {
    if let Some(path) = std::env::var_os("TELMO_SOCKET") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or(Failure::Other("HOME is not set.".into()))?;
    Ok(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
}

fn stream(stop: &AtomicBool, events: &Tx) -> Result<(), Failure> {
    let mut socket = UnixStream::connect(socket_path()?)?;
    socket.write_all(b"audio-stream\n")?;
    socket.set_read_timeout(Some(HEADER_WAIT))?;
    let mut reader = BufReader::new(socket);
    let rate = read_header(&mut reader)?;
    reader.get_ref().set_read_timeout(Some(FRAME))?;

    let mut analyzer = Analyzer::new(rate);
    let mut heard = Instant::now();
    let mut blocked = false;
    let mut last_frame = Instant::now();
    let mut pending = Vec::new();
    let mut chunk = [0u8; 16384];
    while !stopped(stop) {
        match reader.read(&mut chunk) {
            Ok(0) => {
                return Err(Failure::Other(
                    "The Telmo host stopped the visualizer.".into(),
                ));
            }
            Ok(n) => pending.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
        if last_frame.elapsed() < FRAME {
            continue;
        }
        last_frame = Instant::now();
        let samples = take_samples(&mut pending);
        if samples.iter().any(|s| *s != 0.0) {
            heard = Instant::now();
            blocked = false;
        } else if !blocked && heard.elapsed() > SILENT_FOR {
            blocked = true;
            let _ = events.send(Event::VisualizerBlocked);
        }
        analyzer.push(&samples, 1);
        if events.send(Event::Spectrum(analyzer.frame())).is_err() {
            break;
        }
    }
    Ok(())
}

fn read_header(reader: &mut impl BufRead) -> Result<f32, Failure> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let line = line.trim_end();
    if line == "error denied" {
        return Err(Failure::Denied);
    }
    if let Some(message) = line.strip_prefix("error ") {
        return Err(Failure::Other(message.to_string()));
    }
    line.strip_prefix("ok rate=")
        .and_then(|rate| rate.parse::<f32>().ok())
        .filter(|rate| *rate > 0.0)
        .ok_or_else(|| Failure::Other("The Telmo host sent an unreadable audio header.".into()))
}

/// Decodes whole samples and leaves a trailing partial one for next time.
fn take_samples(pending: &mut Vec<u8>) -> Vec<f32> {
    let whole = pending.len() / 4 * 4;
    let samples = pending[..whole]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    pending.drain(..whole);
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_gives_the_rate() {
        assert!(matches!(read_header(&mut "ok rate=44100\n".as_bytes()), Ok(r) if r == 44100.0));
    }

    #[test]
    fn denied_header_is_blocked() {
        assert!(matches!(
            read_header(&mut "error denied\n".as_bytes()),
            Err(Failure::Denied)
        ));
    }

    #[test]
    fn partial_samples_wait_for_the_rest() {
        let mut pending = [1.0f32.to_le_bytes().as_slice(), &[0, 0]].concat();
        assert_eq!(take_samples(&mut pending), vec![1.0]);
        assert_eq!(pending.len(), 2);
    }
}
