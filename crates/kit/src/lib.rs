//! Shared building blocks for the telmo modules: theme, widgets, text input,
//! the event loop, a snapshot cache and a test renderer.

pub mod cache;
pub mod cli;
pub mod hits;
pub mod images;
pub mod input;
pub mod os;
pub mod runtime;
pub mod state;
pub mod test;
pub mod theme;
pub mod widgets;

pub use runtime::{App, Flow, run};
