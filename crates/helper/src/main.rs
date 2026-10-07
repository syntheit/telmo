//! `telmo-helper`: a root daemon that lets the signed Telmo app read and toggle
//! per-network Wi-Fi auto-join. It answers exactly two requests on a Unix socket
//! and only to our own user running a binary signed by our team.

mod known;
mod peer;
mod request;

use request::{MAX_LINE, Request};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

const SOCKET: &str = "/var/run/telmo-helper.sock";
const READ_LIMIT: Duration = Duration::from_secs(2);

struct Config {
    uid: u32,
    team: String,
}

fn main() {
    let config = match parse_args(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("telmo-helper: {message}");
            std::process::exit(2);
        }
    };
    if let Err(message) = serve(&config) {
        eprintln!("telmo-helper: {message}");
        std::process::exit(1);
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let (mut uid, mut team) = (None, None);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value."))?;
        match flag.as_str() {
            "--uid" => {
                uid = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "--uid must be a number.")?,
                )
            }
            "--team-id" => team = Some(value),
            _ => return Err(format!("Unknown flag {flag}.")),
        }
    }
    let team = team.ok_or("--team-id is required.")?;
    if !peer::valid_team_id(&team) {
        return Err("--team-id must be a ten character Apple team ID.".to_string());
    }
    let uid = uid.ok_or("--uid is required.")?;
    if uid == 0 {
        return Err("--uid must not be root.".to_string());
    }
    Ok(Config { uid, team })
}

fn serve(config: &Config) -> Result<(), String> {
    let listener = bind(config.uid)?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(message) = handle(stream, config) {
                    eprintln!("telmo-helper: {message}");
                }
            }
            Err(e) => eprintln!("telmo-helper: accept failed: {e}"),
        }
    }
    Ok(())
}

/// Create the socket owned by the user with mode 0600. The umask makes it
/// private from the moment it exists; nobody else can ever connect.
fn bind(uid: u32) -> Result<UnixListener, String> {
    if let Ok(meta) = std::fs::symlink_metadata(SOCKET) {
        if !meta.file_type().is_socket() {
            return Err(format!("{SOCKET} exists and isn't a socket."));
        }
        std::fs::remove_file(SOCKET)
            .map_err(|e| format!("Couldn't remove the old socket: {e}."))?;
    }
    // SAFETY: umask has no preconditions.
    let old = unsafe { umask(0o177) };
    let listener = UnixListener::bind(SOCKET);
    // SAFETY: as above.
    unsafe { umask(old) };
    let listener = listener.map_err(|e| format!("Couldn't bind {SOCKET}: {e}."))?;
    std::os::unix::fs::chown(SOCKET, Some(uid), None)
        .map_err(|e| format!("Couldn't give the socket to the user: {e}."))?;
    std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Couldn't restrict the socket: {e}."))?;
    Ok(listener)
}

unsafe extern "C" {
    fn umask(mask: u16) -> u16;
}

fn handle(mut stream: UnixStream, config: &Config) -> Result<(), String> {
    peer::verify(&stream, config.uid, &config.team)
        .map_err(|e| format!("Refused a connection: {e}"))?;
    let _ = stream.set_write_timeout(Some(READ_LIMIT));
    let reply = match read_line(&mut stream).and_then(|line| request::parse(&line)) {
        Ok(request) => respond(&request),
        Err(message) => Err(message),
    };
    let text = match reply {
        Ok(text) => text,
        Err(message) => format!("error {message}"),
    };
    stream
        .write_all(format!("{text}\n").as_bytes())
        .map_err(|e| format!("Couldn't reply: {e}."))
}

fn respond(request: &Request) -> Result<String, String> {
    match request {
        Request::AutojoinList => {
            let networks = known::list()?;
            serde_json::to_string(&networks).map_err(|e| e.to_string())
        }
        Request::AutojoinSet { ssid, on } => known::set(ssid, *on).map(|()| "ok".to_string()),
    }
}

/// One newline-terminated line of at most MAX_LINE bytes, read within a
/// 2 s total deadline.
fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let deadline = Instant::now() + READ_LIMIT;
    let mut line = Vec::with_capacity(MAX_LINE);
    let mut byte = [0u8; 1];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("The request took too long.".to_string());
        }
        // macOS refuses this once the peer has closed, but then reads can't block anyway.
        let _ = stream.set_read_timeout(Some(left));
        match stream.read(&mut byte) {
            Ok(0) => return Err("The request has no end of line.".to_string()),
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) if line.len() + 1 >= MAX_LINE => {
                return Err("The request is too long.".to_string());
            }
            Ok(_) => line.push(byte[0]),
            Err(_) => return Err("The request took too long.".to_string()),
        }
    }
    String::from_utf8(line).map_err(|_| "The request isn't UTF-8.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Config, String> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        let config = args(&["--uid", "501", "--team-id", "6NHZWHQX37"]).unwrap();
        assert_eq!((config.uid, config.team.as_str()), (501, "6NHZWHQX37"));
    }

    #[test]
    fn refuses_bad_flags() {
        assert!(args(&[]).is_err());
        assert!(args(&["--uid", "0", "--team-id", "6NHZWHQX37"]).is_err());
        assert!(args(&["--uid", "501", "--team-id", "x\" or true"]).is_err());
        assert!(args(&["--uid", "501", "--team-id", "6NHZWHQX37", "--socket", "/x"]).is_err());
        assert!(args(&["--uid", "abc", "--team-id", "6NHZWHQX37"]).is_err());
    }

    fn line(bytes: &[u8]) -> Result<String, String> {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        b.write_all(bytes).unwrap();
        drop(b);
        read_line(&mut a)
    }

    #[test]
    fn reads_one_bounded_line() {
        assert_eq!(line(b"autojoin-list\n").unwrap(), "autojoin-list");
        assert!(line(b"no newline").is_err());
        let mut long = vec![b'a'; MAX_LINE];
        long.push(b'\n');
        assert!(line(&long).is_err());
        let mut fits = vec![b'a'; MAX_LINE - 1];
        fits.push(b'\n');
        assert!(line(&fits).is_ok());
        assert!(line(b"\xff\n").is_err());
    }

    #[test]
    fn a_silent_peer_times_out() {
        let (mut a, _b) = UnixStream::pair().unwrap();
        let start = Instant::now();
        assert!(read_line(&mut a).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
