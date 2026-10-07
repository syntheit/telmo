//! Listens to everything playing on the Mac, or to the microphone, through
//! the Telmo host app. The "System Audio Recording" and Microphone grants
//! belong to the app, not to this process, so the app runs the Core Audio tap
//! or the input engine and streams mono samples over its socket:
//! `audio-stream` (or `audio-stream mic`), a header line `ok rate=<hz>`, then
//! little-endian f32. Closing the connection stops it.
//!
//! Without the permission the host answers `error denied`. For desktop audio
//! the tap may also deliver silence. Both end in `Event::VisualizerBlocked`;
//! for the microphone a denial is `Event::MicFailed` with what to do.

use super::{FRAME, report_failure, stopped};
use crate::backend::{Event, Tx};
use crate::model::Source;
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

const MIC_DENIED: &str = "Telmo isn't allowed to use the microphone. Turn it on in System Settings > Privacy & Security > Microphone.";

pub fn run(stop: &AtomicBool, events: &Tx, source: Source) {
    match stream(stop, events, source) {
        Ok(()) => {}
        Err(Failure::Denied) if source == Source::Mic => {
            report_failure(events, source, MIC_DENIED.into());
        }
        Err(Failure::Denied) => {
            let _ = events.send(Event::VisualizerBlocked);
        }
        Err(Failure::Other(message)) => report_failure(events, source, message),
    }
}

enum Failure {
    Denied,
    Other(String),
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Other(format!("Couldn't reach the Telmo host: {e}."))
    }
}

fn socket_path() -> Result<PathBuf, Failure> {
    if let Some(path) = std::env::var_os("TELMO_SOCKET") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or(Failure::Other("HOME is not set.".into()))?;
    Ok(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
}

fn stream(stop: &AtomicBool, events: &Tx, source: Source) -> Result<(), Failure> {
    let mut socket = UnixStream::connect(socket_path()?)?;
    let command = match source {
        Source::Desktop => "audio-stream\n",
        Source::Mic => "audio-stream mic\n",
    };
    socket.write_all(command.as_bytes())?;
    socket.set_read_timeout(Some(FRAME))?;
    let mut reader = BufReader::new(socket);
    let Some(rate) = wait_for_header(&mut reader, stop)? else {
        return Ok(());
    };

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
        let mono = take_samples(&mut pending);
        if source == Source::Desktop {
            if mono.iter().any(|s| *s != 0.0) {
                heard = Instant::now();
                blocked = false;
            } else if !blocked && heard.elapsed() > SILENT_FOR {
                blocked = true;
                let _ = events.send(Event::VisualizerBlocked);
            }
            analyzer.push(&mono, 1);
            if events.send(Event::Spectrum(analyzer.frame())).is_err() {
                break;
            }
        }
        let samples = Event::Samples {
            source,
            rate: rate as u32,
            mono,
        };
        if events.send(samples).is_err() {
            break;
        }
    }
    Ok(())
}

/// Reads the header line while watching `stop`. None when stopped first.
fn wait_for_header(
    reader: &mut BufReader<UnixStream>,
    stop: &AtomicBool,
) -> Result<Option<f32>, Failure> {
    let deadline = Instant::now() + HEADER_WAIT;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while !stopped(stop) && Instant::now() < deadline {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => {
                return read_header(&mut line.as_slice().chain(&b"\n"[..])).map(Some);
            }
            Ok(_) => line.push(byte[0]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    if stopped(stop) {
        return Ok(None);
    }
    Err(Failure::Other("The Telmo host did not answer.".into()))
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
