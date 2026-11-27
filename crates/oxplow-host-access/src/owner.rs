//! The running host, as its owner, over its owner socket (`host.sock` in the
//! home, mode 0600: every request on it is the owner's). The CLI's way to
//! the host, and a daemon's: a daemon the host started runs as the same
//! user with the same `OXPLOW_HOME`.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

/// The owner socket of the host for `config_dir` (`serve` makes it).
pub fn owner_socket_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host.sock")
}

/// Why a call to the host did not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OwnerCallError {
    /// No host runs for this home (no socket answers).
    #[error("no host runs for this home ({0})")]
    NoHost(String),
    /// The host answered with a refusal: its own words.
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Failed(String),
}

/// `method path` on the running host, as its owner, with `body` as JSON
/// when given; the JSON answer, or why not. **Blocking.**
#[cfg(unix)]
pub fn owner_call(
    config_dir: &Path,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, OwnerCallError> {
    use std::io::{Read, Write};
    let failed = |e: std::io::Error| OwnerCallError::Failed(e.to_string());
    let socket = owner_socket_path(config_dir);
    let mut stream = std::os::unix::net::UnixStream::connect(&socket)
        .map_err(|e| OwnerCallError::NoHost(format!("{}: {e}", socket.display())))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(failed)?;
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let content_type = if body.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .map_err(failed)?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).map_err(failed)?;
    let answer = String::from_utf8_lossy(&answer);
    let (head, text) = answer
        .split_once("\r\n\r\n")
        .ok_or_else(|| OwnerCallError::Failed("the host's answer was cut short".into()))?;
    let ok = head
        .split_whitespace()
        .nth(1)
        .is_some_and(|code| code.starts_with('2'));
    let value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    if ok {
        Ok(value)
    } else {
        Err(OwnerCallError::Refused(
            value["error"]
                .as_str()
                .map_or_else(|| text.trim().to_string(), str::to_string),
        ))
    }
}

#[cfg(not(unix))]
pub fn owner_call(
    _: &Path,
    _: &str,
    _: &str,
    _: Option<&serde_json::Value>,
) -> Result<serde_json::Value, OwnerCallError> {
    Err(OwnerCallError::NoHost(
        "the host is reached over a Unix socket, which this system lacks".into(),
    ))
}

// The stand-in host is a Unix socket.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    /// A stand-in host on `host.sock`: it answers one request with `status`
    /// and `answer`, and hands back what it was sent.
    fn stand_in(
        home: &Path,
        status: &'static str,
        answer: &'static str,
    ) -> std::thread::JoinHandle<String> {
        let listener = std::os::unix::net::UnixListener::bind(owner_socket_path(home)).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            )
            .unwrap();
            format!("{head}{}", String::from_utf8(body).unwrap())
        })
    }

    #[test]
    fn a_call_sends_its_body_and_reads_the_answer() {
        let home = tempfile::tempdir().unwrap();
        let host = stand_in(home.path(), "200 OK", r#"{"id":"dev-1","name":"Kitchen"}"#);
        let answer = owner_call(
            home.path(),
            "PATCH",
            "/host/devices/dev-1",
            Some(&serde_json::json!({ "name": "Kitchen" })),
        )
        .unwrap();
        assert_eq!(answer["name"], "Kitchen");
        let sent = host.join().unwrap();
        assert!(
            sent.starts_with("PATCH /host/devices/dev-1 HTTP/1.1"),
            "{sent}"
        );
        assert!(sent.contains("Content-Type: application/json"), "{sent}");
        assert!(sent.ends_with(r#"{"name":"Kitchen"}"#), "{sent}");
    }

    #[test]
    fn a_refusal_is_the_hosts_words_and_no_host_is_said() {
        let home = tempfile::tempdir().unwrap();
        assert!(matches!(
            owner_call(home.path(), "GET", "/host/devices", None),
            Err(OwnerCallError::NoHost(_))
        ));
        let host = stand_in(
            home.path(),
            "404 Not Found",
            r#"{"error":"no device dev-9"}"#,
        );
        assert_eq!(
            owner_call(home.path(), "DELETE", "/host/devices/dev-9", None),
            Err(OwnerCallError::Refused("no device dev-9".into()))
        );
        host.join().unwrap();
    }
}
