//! Clickable areas. An app fills a `Hits` while drawing (draw takes `&self`,
//! hence the interior mutability) and asks it what was under a click.

use ratatui::layout::{Position, Rect};
use std::cell::RefCell;

#[derive(Debug)]
pub struct Hits<A>(RefCell<Vec<(Rect, A)>>);

impl<A> Default for Hits<A> {
    fn default() -> Self {
        Self(RefCell::new(Vec::new()))
    }
}

impl<A: Clone> Hits<A> {
    /// Call at the start of every draw.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn add(&self, rect: Rect, action: A) {
        self.0.borrow_mut().push((rect, action));
    }

    /// The action under a cell. Later additions win, so dialogs drawn on
    /// top of the screen take the click.
    pub fn at(&self, column: u16, row: u16) -> Option<A> {
        let position = Position { x: column, y: row };
        self.0
            .borrow()
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(position))
            .map(|(_, action)| action.clone())
    }
}
