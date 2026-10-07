//! Looking a recording up. The app calls a `Recognizer` and gets the answer
//! back as `Event::Recognized`, so the UI never waits for the network.

use crate::backend::{Event, Tx};
use crate::song::Found;

/// Takes 16 kHz mono audio and the number of the listening it belongs to.
pub type Recognizer = fn(Vec<f32>, u64, Tx);

/// Fingerprints and looks up on a task of the running runtime.
pub fn spawn(samples: Vec<f32>, run: u64, events: Tx) {
    tokio::spawn(async move {
        let result = lookup(samples).await;
        let _ = events.send(Event::Recognized { run, result });
    });
}

async fn lookup(samples: Vec<f32>) -> Result<Option<Found>, String> {
    let signature = tokio::task::spawn_blocking(move || telmo_recognize::signature(&samples))
        .await
        .map_err(|_| "Song recognition stopped unexpectedly. Try again.".to_string())?;
    let track = telmo_recognize::recognize(&signature).await?;
    Ok(track.map(Found::from))
}

/// `--mock` and tests: answers at once with a fixed song.
pub fn mock(_samples: Vec<f32>, run: u64, events: Tx) {
    let result = Ok(Some(canned()));
    let _ = events.send(Event::Recognized { run, result });
}

pub fn canned() -> Found {
    Found {
        title: "Sneaky Snitch".into(),
        artist: "Kevin MacLeod".into(),
        album: Some("YouTube Audio Library".into()),
        year: Some("2012".into()),
        shazam_url: Some("https://www.shazam.com/track/0".into()),
        apple_music_url: Some("https://music.apple.com/us/album/0".into()),
        spotify_search_url: "https://open.spotify.com/search/Sneaky%20Snitch%20Kevin%20MacLeod"
            .into(),
    }
}

/// What `--mock` and the snapshot tests show under the card.
pub fn canned_history() -> Vec<Found> {
    let song = |title: &str, artist: &str| Found {
        title: title.into(),
        artist: artist.into(),
        ..canned()
    };
    vec![
        song("Windowlicker", "Aphex Twin"),
        song("Teardrop", "Massive Attack"),
        song("Open Eye Signal", "Jon Hopkins"),
        song("Cirrus", "Bonobo"),
    ]
}
