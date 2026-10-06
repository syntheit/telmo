//! Lets any thread ask the run-loop thread for a new snapshot and wakes the
//! run loop so it can sleep until then.

use block2::RcBlock;
use objc2_core_foundation::{CFRetained, CFRunLoop, CFType, kCFRunLoopDefaultMode};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};

/// CFRunLoopPerformBlock and CFRunLoopWakeUp may be called from any thread.
struct RunLoop(CFRetained<CFRunLoop>);
unsafe impl Send for RunLoop {}
unsafe impl Sync for RunLoop {}

#[derive(Clone)]
pub struct Wake {
    requests: Sender<()>,
    run_loop: Arc<OnceLock<RunLoop>>,
}

impl Wake {
    pub fn new() -> (Wake, Receiver<()>) {
        let (requests, wanted) = mpsc::channel();
        let wake = Wake {
            requests,
            run_loop: Arc::new(OnceLock::new()),
        };
        (wake, wanted)
    }

    /// Call on the thread that will run the loop, before it first runs.
    pub fn attach(&self) {
        if let Some(run_loop) = CFRunLoop::current() {
            let _ = self.run_loop.set(RunLoop(run_loop));
        }
    }

    /// Ask for a snapshot. Safe from any thread.
    pub fn send(&self) {
        let _ = self.requests.send(());
        let Some(RunLoop(run_loop)) = self.run_loop.get() else {
            return;
        };
        // The queued block stops the loop, so `run_in_mode` returns even if it
        // wasn't running yet when this was called.
        let target = run_loop.clone();
        let block = RcBlock::new(move || target.stop());
        let mode: Option<&CFType> = unsafe { kCFRunLoopDefaultMode }.map(|mode| mode.as_ref());
        unsafe { run_loop.perform_block(mode, Some(&block)) };
        run_loop.wake_up();
    }
}
