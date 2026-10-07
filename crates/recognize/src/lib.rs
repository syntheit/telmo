//! Song recognition through Shazam's public discovery endpoint.
//!
//! Ported from SongRec (<https://github.com/marin-m/SongRec>, GPL-3.0) by
//! Marin Moulinier (marin-m): the fingerprinting algorithm, the signature
//! binary format and the request layout all come from that project.

mod audio;
mod shazam;
mod signature;

pub use audio::to_16k_mono;
pub use shazam::{Track, recognize};
pub use signature::{SAMPLE_RATE, Signature, signature};
