//! The event loop shared by every module.
//!
//! The UI redraws only when something happens: a key, a backend event, or a
//! tick (100 ms unless the app asks for another `frame_interval`) while the
//! app says it is animating. An idle popup costs nothing.

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal::supports_keyboard_enhancement};
use futures_util::StreamExt;
use ratatui::Frame;
use std::{io, time::Duration};
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
}

pub async fn run<A: App>(mut app: A, mut events: UnboundedReceiver<A::Event>) -> io::Result<()> {
    let mut terminal = ratatui::init();
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

    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(app.frame_interval());
    let result = async {
        terminal.draw(|f| app.draw(f))?;
        loop {
            let flow = tokio::select! {
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
            if flow == Flow::Quit {
                return Ok(());
            }
            terminal.draw(|f| app.draw(f))?;
        }
    }
    .await;

    let _ = execute!(io::stdout(), DisableMouseCapture);
    if enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}
