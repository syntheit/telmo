//! The event loop shared by every module.
//!
//! The UI redraws only when something happens: a key, a backend event, or a
//! tick (100 ms unless the app asks for another `frame_interval`) while the
//! app says it is animating. An idle popup costs nothing.
//!
//! An app can also run one interactive command (e.g. `sudo`, which needs the
//! terminal for Touch ID or a password): after each event the loop asks
//! `take_command`, leaves the TUI, runs the command with inherited stdio, comes
//! back and reports the result through `command_finished`.

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal::supports_keyboard_enhancement};
use futures_util::StreamExt;
use ratatui::Frame;
use std::{io, process::Command, time::Duration};
use tokio::sync::mpsc::UnboundedReceiver;

pub use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub trait App {
    /// What the backend sends to the UI.
    type Event: Send + 'static;

    fn draw(&self, frame: &mut Frame);
    fn key(&mut self, key: KeyEvent) -> Flow;
    fn event(&mut self, event: Self::Event) -> Flow;

    /// Clicks and scrolling. Apps record clickable areas while drawing
    /// (see `crate::hits::Hits`) and look them up here.
    fn mouse(&mut self, _event: MouseEvent) -> Flow {
        Flow::Continue
    }

    /// Called every `frame_interval` while `animating` returns true.
    fn tick(&mut self) -> Flow {
        Flow::Continue
    }
    /// Time between ticks; read once when the loop starts.
    fn frame_interval(&self) -> Duration {
        Duration::from_millis(100)
    }
    fn animating(&self) -> bool {
        false
    }

    /// A command to run in the popup's own terminal; asked after every event.
    fn take_command(&mut self) -> Option<Command> {
        None
    }
    /// How the command from `take_command` ended.
    fn command_finished(&mut self, _result: io::Result<std::process::ExitStatus>) -> Flow {
        Flow::Continue
    }
}

/// Puts the terminal into popup mode. Returns whether keyboard flags were pushed.
fn enter() -> io::Result<(ratatui::DefaultTerminal, bool)> {
    let terminal = ratatui::init();
    // With the kitty keyboard protocol a lone Esc arrives instantly instead of
    // after the escape-sequence timeout.
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    execute!(io::stdout(), EnableMouseCapture)?;
    Ok((terminal, enhanced))
}

fn leave(enhanced: bool) {
    let _ = execute!(io::stdout(), DisableMouseCapture);
    if enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
}

pub async fn run<A: App>(mut app: A, mut events: UnboundedReceiver<A::Event>) -> io::Result<()> {
    let (mut terminal, mut enhanced) = enter()?;
    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(app.frame_interval());
    let result = async {
        terminal.draw(|f| app.draw(f))?;
        loop {
            let mut flow = tokio::select! {
                key = keys.next() => match key {
                    Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                        let ctrl_c = key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL);
                        if ctrl_c { Flow::Quit } else { app.key(key) }
                    }
                    Some(Ok(Event::Mouse(mouse))) => app.mouse(mouse),
                    Some(Ok(_)) => Flow::Continue,
                    Some(Err(e)) => return Err(e),
                    None => Flow::Quit,
                },
                Some(event) = events.recv() => app.event(event),
                _ = tick.tick(), if app.animating() => app.tick(),
            };
            if flow == Flow::Continue
                && let Some(mut command) = app.take_command()
            {
                // The key stream must not read what the command is waiting for.
                drop(keys);
                leave(enhanced);
                let status = command.status();
                (terminal, enhanced) = enter()?;
                keys = EventStream::new();
                terminal.clear()?;
                flow = app.command_finished(status);
            }
            if flow == Flow::Quit {
                return Ok(());
            }
            terminal.draw(|f| app.draw(f))?;
        }
    }
    .await;

    leave(enhanced);
    result
}
