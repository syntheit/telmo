//! The "What's playing?" dialog's state machine. No I/O: the app feeds it
//! samples and recognition answers, and does what the returned request asks.

use crate::model::Source;
use serde::{Deserialize, Serialize};
use telmo_recognize::{Track, to_16k_mono};

/// Listening time of the first attempt, and of the automatic second one
/// (which keeps the first attempt's audio and adds to it).
const FIRST_SECONDS: usize = 12;
const SECOND_SECONDS: usize = 18;

/// A recognized song, as the card and the history show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub year: Option<String>,
    pub shazam_url: Option<String>,
    pub apple_music_url: Option<String>,
    pub spotify_search_url: String,
    #[serde(default)]
    pub cover_url: Option<String>,
}

impl From<Track> for Found {
    fn from(track: Track) -> Self {
        Self {
            title: track.title,
            artist: track.artist,
            album: track.album,
            year: track.year,
            shazam_url: track.shazam_url,
            apple_music_url: track.apple_music_url,
            spotify_search_url: track.spotify_search_url,
            cover_url: track.cover_url,
        }
    }
}

impl Found {
    /// What `c` copies.
    pub fn label(&self) -> String {
        format!("{} — {}", self.title, self.artist)
    }

    /// "Album · 2012", or whichever of the two is known.
    pub fn details(&self) -> String {
        let parts: Vec<&str> = [&self.album, &self.year]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .collect();
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Listening,
    Recognizing,
    Found(Found),
    NoMatch,
    Failed(String),
}

/// A line under the result: what a key just did, or why it couldn't.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub text: String,
    pub error: bool,
}

impl Note {
    pub fn done(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    pub fn failed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

pub struct Listen {
    pub source: Source,
    pub phase: Phase,
    pub note: Option<Note>,
    /// How loud it is right now, 0.0-1.0, for the meter.
    pub level: f32,
    /// 1, or 2 after the first attempt found nothing.
    attempt: u8,
    rate: u32,
    samples: Vec<f32>,
}

impl Listen {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            phase: Phase::Listening,
            note: None,
            level: 0.0,
            attempt: 1,
            rate: 0,
            samples: Vec::new(),
        }
    }

    /// Whether the capture still has to run. The first attempt keeps
    /// listening while it is looked up, in case a second one is needed.
    pub fn capturing(&self) -> bool {
        match self.phase {
            Phase::Listening => true,
            Phase::Recognizing => self.attempt == 1,
            _ => false,
        }
    }

    /// The first attempt found nothing and this is the second.
    pub fn retrying(&self) -> bool {
        self.attempt == 2
    }

    pub fn target_seconds(&self) -> usize {
        if self.attempt == 1 {
            FIRST_SECONDS
        } else {
            SECOND_SECONDS
        }
    }

    pub fn heard_seconds(&self) -> f32 {
        if self.rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.rate as f32
    }

    /// 0.0-1.0 of the listening time of this attempt.
    pub fn progress(&self) -> f32 {
        (self.heard_seconds() / self.target_seconds() as f32).min(1.0)
    }

    /// Takes mono audio. Returns the 16 kHz audio to look up when there is
    /// enough of it.
    pub fn push(&mut self, rate: u32, mono: &[f32]) -> Option<Vec<f32>> {
        if !self.capturing() || rate == 0 {
            return None;
        }
        self.rate = rate;
        self.samples.extend_from_slice(mono);
        self.level = loudness(mono).max(self.level * 0.7);
        if self.phase == Phase::Listening && self.heard_seconds() >= self.target_seconds() as f32 {
            return Some(self.start_lookup());
        }
        None
    }

    /// The answer to the last request. Returns audio when the first attempt
    /// found nothing and the second can start right away.
    pub fn answer(&mut self, result: Result<Option<Found>, String>) -> Option<Vec<f32>> {
        if self.phase != Phase::Recognizing {
            return None;
        }
        match result {
            Ok(Some(found)) => self.phase = Phase::Found(found),
            Ok(None) if self.attempt == 1 => {
                self.attempt = 2;
                self.phase = Phase::Listening;
                if self.heard_seconds() >= SECOND_SECONDS as f32 {
                    return Some(self.start_lookup());
                }
            }
            Ok(None) => self.phase = Phase::NoMatch,
            Err(message) => self.phase = Phase::Failed(message),
        }
        None
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        if matches!(self.phase, Phase::Listening | Phase::Recognizing) {
            self.phase = Phase::Failed(message.into());
        }
    }

    fn start_lookup(&mut self) -> Vec<f32> {
        self.phase = Phase::Recognizing;
        to_16k_mono(&self.samples, self.rate, 1)
    }
}

/// Root mean square, scaled so ordinary music reaches the upper half.
fn loudness(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    (mean_square.sqrt() * 5.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    fn found() -> Found {
        Found {
            title: "Sneaky Snitch".into(),
            artist: "Kevin MacLeod".into(),
            album: Some("Sneaky Snitch".into()),
            year: Some("2012".into()),
            shazam_url: None,
            apple_music_url: None,
            spotify_search_url: "https://open.spotify.com/search/x".into(),
            cover_url: None,
        }
    }

    fn seconds(listen: &mut Listen, n: usize) -> Option<Vec<f32>> {
        let mut request = None;
        for _ in 0..n {
            request = request.or(listen.push(RATE, &vec![0.1; RATE as usize]));
        }
        request
    }

    #[test]
    fn listens_for_twelve_seconds_then_recognizes() {
        let mut listen = Listen::new(Source::Mic);
        assert!(seconds(&mut listen, 11).is_none());
        assert_eq!(listen.phase, Phase::Listening);
        assert!((listen.progress() - 11.0 / 12.0).abs() < 1e-6);
        let request = listen.push(RATE, &vec![0.1; RATE as usize]);
        assert_eq!(request.map(|a| a.len()), Some(12 * RATE as usize));
        assert_eq!(listen.phase, Phase::Recognizing);
    }

    #[test]
    fn a_match_ends_in_the_result() {
        let mut listen = Listen::new(Source::Desktop);
        seconds(&mut listen, 12);
        assert!(listen.answer(Ok(Some(found()))).is_none());
        assert_eq!(listen.phase, Phase::Found(found()));
        assert!(!listen.capturing());
    }

    #[test]
    fn no_match_tries_again_with_eighteen_seconds() {
        let mut listen = Listen::new(Source::Mic);
        seconds(&mut listen, 13);
        // Audio kept arriving while the first lookup ran.
        assert!(listen.capturing());
        assert!(listen.answer(Ok(None)).is_none());
        assert_eq!(listen.phase, Phase::Listening);
        assert!((listen.progress() - 13.0 / 18.0).abs() < 1e-6);
        let request = seconds(&mut listen, 5);
        assert_eq!(request.map(|a| a.len()), Some(18 * RATE as usize));
        assert!(listen.answer(Ok(None)).is_none());
        assert_eq!(listen.phase, Phase::NoMatch);
        assert!(!listen.capturing());
    }

    #[test]
    fn a_slow_lookup_leaves_enough_audio_for_the_second_try_at_once() {
        let mut listen = Listen::new(Source::Mic);
        seconds(&mut listen, 12);
        seconds(&mut listen, 6);
        assert!(listen.answer(Ok(None)).is_some());
        assert_eq!(listen.phase, Phase::Recognizing);
    }

    #[test]
    fn an_error_shows_its_sentence_without_a_second_try() {
        let mut listen = Listen::new(Source::Mic);
        seconds(&mut listen, 12);
        listen.answer(Err("Couldn't reach the recognition service.".into()));
        assert_eq!(
            listen.phase,
            Phase::Failed("Couldn't reach the recognition service.".into())
        );
    }

    #[test]
    fn answers_and_audio_after_the_end_are_ignored() {
        let mut listen = Listen::new(Source::Mic);
        seconds(&mut listen, 12);
        listen.answer(Ok(Some(found())));
        assert!(listen.push(RATE, &[0.5; 100]).is_none());
        assert!(listen.answer(Ok(None)).is_none());
        assert_eq!(listen.phase, Phase::Found(found()));
    }

    #[test]
    fn failing_only_applies_while_listening() {
        let mut listen = Listen::new(Source::Mic);
        listen.fail("No microphone.");
        assert_eq!(listen.phase, Phase::Failed("No microphone.".into()));
        listen.fail("Other.");
        assert_eq!(listen.phase, Phase::Failed("No microphone.".into()));
    }

    #[test]
    fn level_follows_the_sound_and_falls_back() {
        let mut listen = Listen::new(Source::Mic);
        listen.push(RATE, &[0.2; 800]);
        let loud = listen.level;
        assert!(loud > 0.5);
        listen.push(RATE, &[0.0; 800]);
        assert!(listen.level < loud);
    }

    #[test]
    fn details_skip_what_is_missing() {
        let mut song = found();
        assert_eq!(song.details(), "Sneaky Snitch · 2012");
        song.album = None;
        assert_eq!(song.details(), "2012");
        song.year = None;
        assert_eq!(song.details(), "");
        assert_eq!(song.label(), "Sneaky Snitch — Kevin MacLeod");
    }
}
