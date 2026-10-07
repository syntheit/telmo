//! Listens to what is playing (or to the microphone) and sends raw mono
//! samples, and for desktop audio spectrum frames, to the UI. Runs on its own
//! thread, only between `start` and drop.

use crate::backend::{Event, Tx};
use crate::model::Source;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub mod mock;

/// About 30 frames a second.
const FRAME: Duration = Duration::from_millis(33);

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn start(mock: bool, source: Source, events: Tx) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("telmo-capture".into())
            .spawn(move || {
                if mock {
                    return mock::run(&flag, &events, source);
                }
                #[cfg(target_os = "linux")]
                linux::run(&flag, &events, source);
                #[cfg(target_os = "macos")]
                macos::run(&flag, &events, source);
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for Capture {
    /// Waits for the thread, so the system audio is released before we exit.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed)
}

/// Reports a failure where the user will see it: the microphone's in the
/// song dialog, the desktop audio's as a toast.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn report_failure(events: &Tx, source: Source, message: String) {
    let event = match source {
        Source::Mic => Event::MicFailed(message),
        Source::Desktop => Event::Failed(message),
    };
    let _ = events.send(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Event;
    use std::time::Instant;
    use tokio::sync::mpsc::unbounded_channel;

    /// Listens to the real system for 5 seconds and prints the loudest bars.
    /// Play something meanwhile: `cargo test -p telmo-sound -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn prints_the_real_spectrum() {
        let (tx, mut rx) = unbounded_channel();
        let capture = Capture::start(false, Source::Desktop, tx);
        let (mut frames, mut loudest) = (0, 0.0f32);
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            match rx.try_recv() {
                Ok(Event::Spectrum(bars)) => {
                    frames += 1;
                    let peak = bars.iter().copied().fold(0.0, f32::max);
                    if frames % 15 == 0 || peak > loudest {
                        let line: String =
                            bars.iter().step_by(2).map(|b| format!("{b:.1} ")).collect();
                        println!("peak {peak:.2}: {line}");
                    }
                    loudest = loudest.max(peak);
                }
                Ok(Event::Samples { .. }) => {}
                Ok(Event::VisualizerBlocked) => println!("blocked by the system"),
                Ok(Event::Failed(message)) => println!("failed: {message}"),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        drop(capture);
        println!("{frames} frames, loudest bar {loudest:.2}");
    }

    /// Listens to what is playing for 14 seconds and looks the song up for
    /// real: `cargo test -p telmo-sound -- --ignored --nocapture identifies`.
    #[test]
    #[ignore]
    fn identifies_what_is_playing() {
        let (tx, mut rx) = unbounded_channel();
        let capture = Capture::start(false, Source::Desktop, tx);
        let (mut heard, mut rate) = (Vec::new(), 0);
        let end = Instant::now() + Duration::from_secs(14);
        while Instant::now() < end {
            match rx.try_recv() {
                Ok(Event::Samples { rate: r, mono, .. }) => {
                    rate = r;
                    heard.extend(mono);
                }
                Ok(Event::Failed(message)) => panic!("{message}"),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        drop(capture);
        println!("{} samples at {rate} Hz", heard.len());
        let audio = telmo_recognize::to_16k_mono(&heard, rate, 1);
        let signature = telmo_recognize::signature(&audio);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        match runtime.block_on(telmo_recognize::recognize(&signature)) {
            Ok(Some(track)) => println!("RESULT {} — {}", track.title, track.artist),
            Ok(None) => println!("RESULT no match"),
            Err(message) => println!("RESULT error: {message}"),
        }
    }

    #[test]
    fn mock_capture_stops_when_dropped() {
        let (tx, mut rx) = unbounded_channel();
        drop(Capture::start(true, Source::Desktop, tx));
        while rx.try_recv().is_ok() {}
        thread::sleep(FRAME * 3);
        assert!(rx.try_recv().is_err());
    }
}
