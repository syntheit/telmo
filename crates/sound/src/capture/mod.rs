//! Listens to what is playing and sends spectrum frames to the UI. Runs on
//! its own thread, only between `start` and drop.

use crate::backend::Tx;
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
    pub fn start(mock: bool, events: Tx) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("telmo-visualizer".into())
            .spawn(move || {
                if mock {
                    return mock::run(&flag, &events);
                }
                #[cfg(target_os = "linux")]
                linux::run(&flag, &events);
                #[cfg(target_os = "macos")]
                macos::run(&flag, &events);
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
        let capture = Capture::start(false, tx);
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
                Ok(Event::VisualizerBlocked) => println!("blocked by the system"),
                Ok(Event::Failed(message)) => println!("failed: {message}"),
                _ => thread::sleep(Duration::from_millis(5)),
            }
        }
        drop(capture);
        println!("{frames} frames, loudest bar {loudest:.2}");
    }

    #[test]
    fn mock_capture_stops_when_dropped() {
        let (tx, mut rx) = unbounded_channel();
        drop(Capture::start(true, tx));
        while rx.try_recv().is_ok() {}
        thread::sleep(FRAME * 3);
        assert!(rx.try_recv().is_err());
    }
}
