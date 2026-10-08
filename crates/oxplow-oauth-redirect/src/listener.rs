//! One loopback socket a sign-in's redirect comes back to (RFC 8252
//! §7.3): it reads requests, hands over `GET /callback`, and answers the
//! browser.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The most bytes of a request's line and headers read.
const MAX_REQUEST_HEAD: usize = 16 * 1024;
/// How long one connection may take to send its request.
const REQUEST_READ_WAIT: Duration = Duration::from_secs(10);
/// How many connections are read at once while waiting for the redirect.
const MAX_READING: usize = 16;

/// An HTTP request as far as the listener reads one: the redirect is a
/// `GET`, so its line is all that matters.
struct HttpRequest {
    method: String,
    /// The path and query.
    target: String,
}

/// Listening on loopback for a sign-in's redirect.
pub struct RedirectListener {
    listener: TcpListener,
    port: u16,
    /// Connections being read, each within [`REQUEST_READ_WAIT`]: one that
    /// sends nothing (another process, a browser's idle socket) never
    /// holds up the real redirect.
    reading: tokio::task::JoinSet<(TcpStream, Option<HttpRequest>)>,
}

/// A request for `/callback`: what it asked for, and the browser to
/// answer.
pub struct Redirect {
    /// The path and query, as the browser sent them.
    pub target: String,
    conn: TcpStream,
}

impl RedirectListener {
    /// Listen on `127.0.0.1:port` (any free port for `0`).
    pub async fn bind(port: u16) -> std::io::Result<RedirectListener> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        Ok(RedirectListener {
            listener,
            port,
            reading: tokio::task::JoinSet::new(),
        })
    }

    /// The port it listens on: where the redirect must come back.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The next `GET /callback`. Anything else is answered — `404`, or
    /// `400` for what isn't a request — and skipped.
    pub async fn next(&mut self) -> std::io::Result<Redirect> {
        loop {
            let (mut conn, request) = tokio::select! {
                accepted = self.listener.accept(), if self.reading.len() < MAX_READING => {
                    let (mut conn, _) = accepted?;
                    self.reading.spawn(async move {
                        let request =
                            tokio::time::timeout(REQUEST_READ_WAIT, read_request(&mut conn))
                                .await
                                .ok()
                                .and_then(Result::ok);
                        (conn, request)
                    });
                    continue;
                }
                Some(read) = self.reading.join_next() => match read {
                    Ok(read) => read,
                    Err(_) => continue,
                },
            };
            let Some(request) = request else {
                let _ = respond(
                    &mut conn,
                    "400 Bad Request",
                    "That isn't a request oxplow reads.",
                )
                .await;
                continue;
            };
            let path = request
                .target
                .split_once('?')
                .map_or(request.target.as_str(), |(p, _)| p);
            if request.method != "GET" || path != "/callback" {
                let _ = respond(&mut conn, "404 Not Found", "Nothing here.").await;
                continue;
            }
            return Ok(Redirect {
                target: request.target,
                conn,
            });
        }
    }
}

impl Redirect {
    /// Answer the browser with a plain-text page: `200` when `ok`, `400`
    /// otherwise.
    pub async fn answer(mut self, ok: bool, text: &str) {
        let status = if ok { "200 OK" } else { "400 Bad Request" };
        let _ = respond(&mut self.conn, status, text).await;
    }
}

fn too_large(what: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("the request's {what} is too large"),
    )
}

/// Read one HTTP/1.1 request's line and headers, within
/// [`MAX_REQUEST_HEAD`] — a line that never ends is an error, not a wait.
/// A body is never read: the redirect has none.
async fn read_request(conn: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let mut reader = BufReader::new(conn);
    let mut head = (&mut reader).take(MAX_REQUEST_HEAD as u64);
    let mut next_line = async || -> std::io::Result<String> {
        let mut line = String::new();
        let n = head.read_line(&mut line).await?;
        // Cut short by the bound: no end of line within it.
        if n > 0 && !line.ends_with('\n') {
            return Err(too_large("head"));
        }
        Ok(line)
    };
    let line = next_line().await?;
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or_default().to_string(),
        parts.next().unwrap_or_default().to_string(),
    );
    while !next_line().await?.trim().is_empty() {}
    Ok(HttpRequest { method, target })
}

/// A plain-text page for the person's browser — never sniffed as
/// anything else, never cached (tsk904).
async fn respond(conn: &mut TcpStream, status: &str, text: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         X-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    conn.write_all(head.as_bytes()).await?;
    conn.write_all(text.as_bytes()).await?;
    conn.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(port: u16, target: &str) -> (u16, String) {
        let resp = reqwest::get(format!("http://127.0.0.1:{port}{target}"))
            .await
            .unwrap();
        (resp.status().as_u16(), resp.text().await.unwrap())
    }

    /// The redirect is handed over with its query; anything else is
    /// answered and skipped, and the listener waits on.
    #[tokio::test]
    async fn only_a_callback_is_handed_over() {
        let mut listener = RedirectListener::bind(0).await.unwrap();
        let port = listener.port();
        let browser = tokio::spawn(async move {
            let other = get(port, "/favicon.ico").await;
            let callback = get(port, "/callback?code=c&state=s").await;
            (other, callback)
        });
        let redirect = listener.next().await.unwrap();
        assert_eq!(redirect.target, "/callback?code=c&state=s");
        redirect.answer(true, "Signed in.").await;
        let (other, callback) = browser.await.unwrap();
        assert_eq!(other.0, 404);
        assert_eq!(callback, (200, "Signed in.".to_string()));
    }

    /// tsk904: the page is plain text the browser neither sniffs nor
    /// keeps.
    #[tokio::test]
    async fn the_page_is_never_sniffed_or_cached() {
        let mut listener = RedirectListener::bind(0).await.unwrap();
        let port = listener.port();
        let browser = tokio::spawn(async move {
            reqwest::get(format!("http://127.0.0.1:{port}/callback?state=s"))
                .await
                .unwrap()
        });
        listener
            .next()
            .await
            .unwrap()
            .answer(true, "Signed in.")
            .await;
        let resp = browser.await.unwrap();
        assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
        assert_eq!(resp.headers()["cache-control"], "no-store");
    }

    /// tsk825: a connection that sends nothing (another process, a
    /// browser's idle socket) doesn't hold up the real redirect.
    #[tokio::test]
    async fn an_idle_connection_doesnt_hold_up_the_redirect() {
        let mut listener = RedirectListener::bind(0).await.unwrap();
        let port = listener.port();
        let _idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut half = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        // A head that never ends.
        half.write_all(b"GET /callback HTTP/1.1\r\nHost: a\r\n")
            .await
            .unwrap();
        let browser = tokio::spawn(async move { get(port, "/callback?state=s").await });
        // Returns as soon as the redirect arrives; the budget only bounds a
        // real hang, so it allows for a loaded machine.
        let redirect = tokio::time::timeout(Duration::from_secs(30), listener.next())
            .await
            .expect("the redirect is handed over")
            .unwrap();
        assert_eq!(redirect.target, "/callback?state=s");
        redirect.answer(false, "Not this one.").await;
        assert_eq!(browser.await.unwrap().0, 400);
    }

    /// tsk825: a request's head is bounded: a line with no end is refused
    /// rather than read.
    #[tokio::test]
    async fn a_request_is_read_within_its_bounds() {
        async fn read_after(bytes: Vec<u8>) -> std::io::Result<HttpRequest> {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let writer = tokio::spawn(async move {
                let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                let _ = c.write_all(&bytes).await;
                // Keep the connection open while the reader decides.
                tokio::time::sleep(Duration::from_secs(2)).await;
            });
            let (mut conn, _) = listener.accept().await.unwrap();
            let out = read_request(&mut conn).await;
            writer.abort();
            out
        }
        let endless = vec![b'a'; MAX_REQUEST_HEAD + 10];
        assert!(read_after(endless).await.is_err());
        let fine = read_after(b"GET /callback?x=1 HTTP/1.1\r\nHost: a\r\n\r\n".to_vec())
            .await
            .unwrap();
        assert_eq!(
            (fine.method.as_str(), fine.target.as_str()),
            ("GET", "/callback?x=1")
        );
    }
}
