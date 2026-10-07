//! What the backend reports. The UI never sees raw frames.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Why nothing can be measured, as a sentence for the user.
    pub unavailable: Option<String>,
    /// A finger (or the object) is on the trackpad right now.
    pub touching: bool,
    /// Smoothed trackpad pressure in grams, including the finger.
    pub grams: f32,
    /// The reading has not moved for a while.
    pub stable: bool,
}
