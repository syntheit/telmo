//! The event loop shared by every module.
//!
//! The UI redraws only when something happens: a key, a backend event, or a
//! 100 ms tick while the app says it is animating. An idle popup costs nothing.

use crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal::supports_keyboard_enhancement};
use futures_util::StreamExt;
use ratatui::Frame;
use std::{io, time::Duration};
use tokio::sync::mpsc::UnboundedReceiver;

pub use crossterm::event::KeyEvent;

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

    /// Called every 100 ms while `animating` returns true.
    fn tick(&mut self) -> Flow {
        Flow::Continue
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

    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
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

    if enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}
