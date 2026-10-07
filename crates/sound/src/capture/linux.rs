//! Records the default output's monitor source (what is playing) or the
//! default microphone through libpulse (PipeWire's pulse layer has both). The
//! mainloop is polled, so stopping is prompt.

use super::{FRAME, report_failure, stopped};
use crate::backend::{Event, Tx};
use crate::model::Source;
use crate::spectrum::Analyzer;
use libpulse_binding::{
    context::{Context, FlagSet as ContextFlags, State},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format, Spec},
    stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream},
};
use std::{
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const POLL: Duration = Duration::from_millis(5);

pub fn run(stop: &AtomicBool, events: &Tx, source: Source) {
    if let Err(message) = record(stop, events, source) {
        report_failure(events, source, message);
    }
}

fn record(stop: &AtomicBool, events: &Tx, source: Source) -> Result<(), String> {
    let (device, channels, unavailable) = match source {
        Source::Desktop => (
            Some("@DEFAULT_MONITOR@"),
            2,
            "Couldn't listen to the sound server, so there is no visualizer.",
        ),
        Source::Mic => (
            None,
            1,
            "Couldn't listen to the sound server, so the microphone is not available.",
        ),
    };
    // Ask the server for 10 ms fragments.
    let fragment_bytes = RATE / 100 * channels as u32 * 4;
    let unavailable = || unavailable;
    let mut mainloop = Mainloop::new().ok_or_else(unavailable)?;
    let mut context = Context::new(&mainloop, "telmo-visualizer").ok_or_else(unavailable)?;
    context
        .connect(None, ContextFlags::NOFLAGS, None)
        .map_err(|_| unavailable())?;
    wait(&mut mainloop, stop, || match context.get_state() {
        State::Ready => Some(true),
        State::Failed | State::Terminated => Some(false),
        _ => None,
    })
    .filter(|ready| *ready)
    .ok_or_else(unavailable)?;

    let spec = Spec {
        format: Format::F32le,
        channels,
        rate: RATE,
    };
    let mut stream =
        Stream::new(&mut context, "visualizer", &spec, None).ok_or_else(unavailable)?;
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: fragment_bytes,
    };
    stream
        .connect_record(
            device,
            Some(&attr),
            StreamFlags::ADJUST_LATENCY | StreamFlags::DONT_MOVE,
        )
        .map_err(|_| unavailable())?;
    wait(&mut mainloop, stop, || match stream.get_state() {
        StreamState::Ready => Some(true),
        StreamState::Failed | StreamState::Terminated => Some(false),
        _ => None,
    })
    .filter(|ready| *ready)
    .ok_or_else(unavailable)?;

    let mut analyzer = Analyzer::new(RATE as f32);
    let mut heard = Vec::new();
    let mut last_frame = Instant::now();
    while !stopped(stop) {
        if !matches!(mainloop.iterate(false), IterateResult::Success(_)) {
            break;
        }
        let mut idle = true;
        loop {
            match stream.peek() {
                Ok(PeekResult::Data(bytes)) => {
                    idle = false;
                    let samples: Vec<f32> = bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_le_bytes(*b))
                        .collect();
                    analyzer.push(&samples, usize::from(channels));
                    heard.extend(
                        samples
                            .chunks_exact(usize::from(channels))
                            .map(|frame| frame.iter().sum::<f32>() / f32::from(channels)),
                    );
                }
                Ok(PeekResult::Hole(_)) => {}
                Ok(PeekResult::Empty) | Err(_) => break,
            }
            if stream.discard().is_err() {
                break;
            }
        }
        if last_frame.elapsed() >= FRAME {
            last_frame = Instant::now();
            if source == Source::Desktop && events.send(Event::Spectrum(analyzer.frame())).is_err()
            {
                break;
            }
            let mono = std::mem::take(&mut heard);
            if events
                .send(Event::Samples {
                    source,
                    rate: RATE,
                    mono,
                })
                .is_err()
            {
                break;
            }
        }
        if idle {
            thread::sleep(POLL);
        }
    }
    let _ = stream.disconnect();
    context.disconnect();
    Ok(())
}

/// Runs the mainloop until `done` has an answer. None when stopped or broken.
fn wait<T>(
    mainloop: &mut Mainloop,
    stop: &AtomicBool,
    mut done: impl FnMut() -> Option<T>,
) -> Option<T> {
    while !stopped(stop) {
        if !matches!(mainloop.iterate(false), IterateResult::Success(_)) {
            return None;
        }
        if let Some(answer) = done() {
            return Some(answer);
        }
        thread::sleep(POLL);
    }
    None
}
