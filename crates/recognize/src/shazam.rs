//! Shazam recognition request, as sent by SongRec's `communication.rs`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::signature::Signature;

const USER_AGENT: &str = "Dalvik/2.1.0 (Linux; U; Android 9; SM-G960F Build/PPR1.180610.011)";

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub year: Option<String>,
    pub cover_url: Option<String>,
    pub shazam_url: Option<String>,
    pub apple_music_url: Option<String>,
    pub spotify_search_url: String,
}

/// Look the signature up. `Ok(None)` means Shazam doesn't know the song.
pub async fn recognize(sig: &Signature) -> Result<Option<Track>, String> {
    let unreachable = || "Couldn't reach the recognition service.".to_string();

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| unreachable())?;
    let response = client
        .post(request_url())
        .header("Content-Language", "en_US")
        .json(&request_body(sig))
        .send()
        .await
        .map_err(|_| unreachable())?;

    match response.status().as_u16() {
        200 => {}
        429 => {
            return Err(
                "The recognition service is rate-limiting this network. Try again later.".into(),
            );
        }
        _ => return Err("The recognition service returned an error. Try again later.".into()),
    }
    let json: Value = response
        .json()
        .await
        .map_err(|_| "The recognition service sent an unreadable answer.".to_string())?;
    Ok(parse_track(&json))
}

fn request_body(sig: &Signature) -> Value {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u32);
    json!({
        "geolocation": { "altitude": 300, "latitude": 45, "longitude": 2 },
        "signature": {
            "samplems": sig.duration_ms(),
            "timestamp": now_ms,
            "uri": sig.to_uri(),
        },
        "timestamp": now_ms,
        "timezone": "Europe/Paris",
    })
}

fn request_url() -> String {
    format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}\
         ?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
        pseudo_uuid(1).to_uppercase(),
        pseudo_uuid(2),
    )
}

/// The service only wants unique-looking ids, so the clock is random enough.
fn pseudo_uuid(salt: u128) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835) ^ salt.rotate_left(64);
    let h = format!("{n:032x}");
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

fn parse_track(json: &Value) -> Option<Track> {
    let track = json.get("track")?;
    let text = |v: &Value| v.as_str().map(str::to_owned);
    let title = text(&track["title"])?;
    let artist = text(&track["subtitle"])?;

    let metadata = |name: &str| {
        track["sections"]
            .as_array()?
            .iter()
            .find(|s| s["type"] == "SONG")?["metadata"]
            .as_array()?
            .iter()
            .find(|m| m["title"] == name)
            .and_then(|m| text(&m["text"]))
    };

    Some(Track {
        spotify_search_url: format!(
            "https://open.spotify.com/search/{}",
            percent_encode(&format!("{title} {artist}"))
        ),
        album: metadata("Album"),
        year: metadata("Released"),
        cover_url: text(&track["images"]["coverart"]),
        shazam_url: text(&track["url"]),
        apple_music_url: apple_music_url(track),
        title,
        artist,
    })
}

/// Apple Music page, rebuilt from the Android `intent://` deep link.
fn apple_music_url(track: &Value) -> Option<String> {
    let uri = track["hub"]["options"]
        .as_array()?
        .iter()
        .flat_map(|o| o["actions"].as_array().into_iter().flatten())
        .find(|a| a["name"] == "hub:applemusic:deeplink")?["uri"]
        .as_str()?;
    let path = uri.strip_prefix("intent://")?.split('#').next()?;
    Some(format!("https://{path}"))
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_track_means_no_match() {
        assert_eq!(parse_track(&json!({ "matches": [] })), None);
    }

    #[test]
    fn parses_a_match() {
        let json = json!({ "track": {
            "title": "Gymnopédie No.1",
            "subtitle": "Erik Satie",
            "url": "https://www.shazam.com/track/1",
            "images": { "coverart": "https://img/cover.jpg" },
            "sections": [
                { "type": "VIDEO" },
                { "type": "SONG", "metadata": [
                    { "title": "Album", "text": "Piano Works" },
                    { "title": "Released", "text": "1888" },
                ]},
            ],
            "hub": { "options": [ { "actions": [
                {
                    "name": "hub:applemusic:deeplink",
                    "uri": "intent://music.apple.com/us/album/x/1?i=2#Intent;scheme=http;end",
                },
            ]}]},
        }});
        let track = parse_track(&json).unwrap();
        assert_eq!(track.title, "Gymnopédie No.1");
        assert_eq!(track.album.as_deref(), Some("Piano Works"));
        assert_eq!(track.year.as_deref(), Some("1888"));
        assert_eq!(track.cover_url.as_deref(), Some("https://img/cover.jpg"));
        assert_eq!(
            track.apple_music_url.as_deref(),
            Some("https://music.apple.com/us/album/x/1?i=2")
        );
        assert_eq!(
            track.spotify_search_url,
            "https://open.spotify.com/search/Gymnop%C3%A9die%20No.1%20Erik%20Satie"
        );
    }

    #[test]
    fn uuids_look_like_uuids() {
        let id = pseudo_uuid(1);
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
    }
}
