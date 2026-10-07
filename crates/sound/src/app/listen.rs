//! The "What's playing?" dialog: which capture runs, what its keys do, and
//! how recognition answers come back.

use super::{App, Dialog};
use crate::history;
use crate::model::Source;
use crate::song::{Found, Listen, Note, Phase};
use ratatui::crossterm::event::KeyCode;

const DESKTOP_DENIED: &str = "Telmo isn't allowed to hear the desktop audio. Allow System Audio Recording for Telmo in System Settings, or press m to use the microphone.";

impl App {
    #[cfg(test)]
    pub fn set_recognizer(&mut self, recognizer: crate::identify::Recognizer) {
        self.recognizer = recognizer;
    }

    #[cfg(test)]
    pub fn mic_running(&self) -> bool {
        self.mic.is_some()
    }

    pub fn listen(&self) -> Option<&Listen> {
        match &self.dialog {
            Some(Dialog::Song(listen)) => Some(listen),
            _ => None,
        }
    }

    /// Desktop audio when something is playing, otherwise the room.
    pub(super) fn open_song(&mut self) {
        let source = if self.playing() {
            Source::Desktop
        } else {
            Source::Mic
        };
        self.begin_listening(source);
    }

    fn begin_listening(&mut self, source: Source) {
        self.run += 1;
        self.dialog = Some(Dialog::Song(Box::new(Listen::new(source))));
        if source == Source::Desktop {
            // A new try may ask the system for permission again.
            self.blocked = false;
        }
        self.sync_listening();
    }

    pub(super) fn close_dialog(&mut self) {
        self.dialog = None;
        self.run += 1;
        self.sync_listening();
    }

    pub(super) fn listening_to(&self, source: Source) -> bool {
        self.listen()
            .is_some_and(|l| l.source == source && l.capturing())
    }

    /// Runs the microphone only while the dialog needs it, and lets the
    /// desktop capture follow along.
    fn sync_listening(&mut self) {
        if !self.listening_to(Source::Mic) {
            self.mic = None;
        } else if self.mic.is_none() {
            self.mic = Some(self.start_capture(Source::Mic));
        }
        self.sync_capture();
    }

    fn recognize(&mut self, audio: Vec<f32>) {
        (self.recognizer)(audio, self.run, self.events.clone());
    }

    pub(super) fn heard(&mut self, source: Source, rate: u32, mono: &[f32]) {
        let request = match &mut self.dialog {
            Some(Dialog::Song(listen)) if listen.source == source => listen.push(rate, mono),
            _ => None,
        };
        if let Some(audio) = request {
            self.recognize(audio);
        }
    }

    pub(super) fn recognized(&mut self, run: u64, result: Result<Option<Found>, String>) {
        if run != self.run {
            return;
        }
        let Some(Dialog::Song(listen)) = &mut self.dialog else {
            return;
        };
        let next = listen.answer(result);
        if let Phase::Found(found) = &listen.phase {
            history::remember(&mut self.history, found.clone());
            if let Some(path) = &self.history_path {
                history::save(path, &self.history);
            }
        }
        if let Some(audio) = next {
            self.recognize(audio);
        }
        self.sync_listening();
    }

    /// The capture can't deliver. Returns false when the dialog isn't listening
    /// to that source, so the caller can report it another way.
    pub(super) fn listening_failed(&mut self, source: Source, message: &str) -> bool {
        if !self.listening_to(source) {
            return false;
        }
        if let Some(Dialog::Song(listen)) = &mut self.dialog {
            listen.fail(message);
        }
        self.sync_listening();
        true
    }

    pub(super) fn visualizer_blocked(&mut self) {
        self.blocked = true;
        // Silence while nothing plays is not a refusal.
        if self.playing() {
            self.listening_failed(Source::Desktop, DESKTOP_DENIED);
        }
    }

    pub(super) fn song_key(&mut self, code: KeyCode) {
        let Some(Dialog::Song(listen)) = &self.dialog else {
            return;
        };
        let source = listen.source;
        let done = !matches!(listen.phase, Phase::Listening | Phase::Recognizing);
        let found = match &listen.phase {
            Phase::Found(found) => Some(found.clone()),
            _ => None,
        };
        match code {
            KeyCode::Char('m') => self.begin_listening(match source {
                Source::Desktop => Source::Mic,
                Source::Mic => Source::Desktop,
            }),
            KeyCode::Char('r') if done => self.begin_listening(source),
            KeyCode::Char(key @ ('c' | 'o' | 'a' | 's')) => {
                if let Some(found) = found {
                    self.song_action(key, &found);
                }
            }
            _ => {}
        }
    }

    fn song_action(&mut self, key: char, found: &Found) {
        let note = match key {
            'c' => {
                if !self.mock {
                    telmo_kit::os::copy(&found.label());
                }
                Note::done("Copied to the clipboard.")
            }
            _ => match page_for(key, found) {
                Some(url) => match self.open(url) {
                    Ok(()) => return,
                    Err(message) => Note::failed(message),
                },
                None => Note::failed("There is no page for this song."),
            },
        };
        if let Some(Dialog::Song(listen)) = &mut self.dialog {
            listen.note = Some(note);
        }
    }

    fn open(&self, url: &str) -> Result<(), String> {
        if self.mock {
            return Ok(());
        }
        telmo_kit::os::open(url)
    }
}

/// The page `o` (Shazam), `a` (Apple Music) or `s` (Spotify search) opens.
pub fn page_for(key: char, found: &Found) -> Option<&str> {
    match key {
        'o' => found.shazam_url.as_deref(),
        'a' => found.apple_music_url.as_deref(),
        's' => Some(&found.spotify_search_url),
        _ => None,
    }
}
