//! The Telmo host app (macOS) listens on a Unix socket; modules send it one
//! line and read the reply until it closes the connection.

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

/// `$TELMO_SOCKET`, else the host's default socket under `$HOME`.
pub fn socket_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("TELMO_SOCKET") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set.")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
}

/// Sends one command and returns the reply. A reply starting with `error `
/// becomes `Err` with the rest of the line. `read` bounds the wait for the
/// reply; `write` bounds sending.
pub fn command(line: &str, read: Duration, write: Duration) -> Result<String, String> {
    let unreachable = |e: std::io::Error| format!("Couldn't reach the Telmo host: {e}.");
    let mut stream = UnixStream::connect(socket_path()?).map_err(unreachable)?;
    stream.set_read_timeout(Some(read)).map_err(unreachable)?;
    stream.set_write_timeout(Some(write)).map_err(unreachable)?;
    stream
        .write_all(format!("{line}\n").as_bytes())
        .map_err(unreachable)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(unreachable)?;
    parse_reply(&reply)
}

fn parse_reply(reply: &str) -> Result<String, String> {
    let reply = reply.trim_end_matches('\n');
    match reply.strip_prefix("error ") {
        Some(message) => Err(message.to_string()),
        None => Ok(reply.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn replies_split_into_ok_and_error() {
        assert_eq!(parse_reply("ok 1\n"), Ok("ok 1".to_string()));
        assert_eq!(parse_reply("error no\n"), Err("no".to_string()));
        assert_eq!(parse_reply(""), Ok(String::new()));
    }

    #[test]
    fn talks_over_the_socket() {
        let dir = std::env::temp_dir().join(format!("telmo-kit-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = [0u8; 64];
            let n = stream.read(&mut line).unwrap();
            assert_eq!(&line[..n], b"ping\n");
            stream.write_all(b"pong\n").unwrap();
        });
        // SAFETY: only this test touches TELMO_SOCKET in this crate.
        unsafe { std::env::set_var("TELMO_SOCKET", &path) };
        let limit = Duration::from_secs(2);
        assert_eq!(command("ping", limit, limit), Ok("pong".to_string()));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
