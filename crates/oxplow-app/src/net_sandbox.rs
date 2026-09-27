//! Network allowlist for `exec` sources (tsk324).
//!
//! A source declares the hosts it talks to (`network: [api.github.com,
//! "*.githubusercontent.com"]`); the list is part of what a person approves.
//! On macOS it's enforced:
//!
//! - the program runs under `sandbox-exec` with [`PROFILE`], which denies
//!   all outbound network except unix sockets and localhost;
//! - a per-run [`EgressProxy`] on localhost forwards only to declared hosts
//!   (HTTP `CONNECT` for HTTPS, absolute-form requests for plain HTTP), and
//!   the program gets `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` pointing at
//!   it.
//!
//! So the proxy is the only way out, and it only goes where the source said
//! it would. A source with no `network` gets no egress at all. Elsewhere
//! [`enforced`] is false and the approval says so. The sandbox still lets a
//! program reach other localhost ports. See `.context/semantic-layer.md`.

use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The `sandbox-exec` binary on macOS.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Deny outbound network except unix sockets (the system resolver, XPC)
/// and localhost (the egress proxy).
pub const PROFILE: &str = "(version 1)(allow default)(deny network-outbound)\
(allow network-outbound (remote unix-socket))\
(allow network-outbound (remote ip \"localhost:*\"))";

/// Whether this OS enforces a source's `network` list.
pub fn enforced() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).exists()
}

/// A `network` entry: a lowercase host name, optionally `*.`-prefixed to
/// allow its subdomains. No scheme, port or path.
pub fn valid_host_pattern(p: &str) -> bool {
    let host = p.strip_prefix("*.").unwrap_or(p);
    !host.is_empty()
        && (host.contains('.') || host == "localhost")
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

/// Whether `host` is allowed by `patterns`: an exact match, or a subdomain
/// of a `*.` pattern (not the bare domain itself).
pub fn host_allowed(host: &str, patterns: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    patterns.iter().any(|p| match p.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1),
        None => host == *p,
    })
}

/// A running per-source egress proxy; stops when dropped.
pub struct EgressProxy {
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for EgressProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl EgressProxy {
    /// Listen on a random localhost port, forwarding only to `allowed`.
    pub async fn start(allowed: Vec<String>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let allowed = Arc::new(allowed);
        let task = tokio::spawn(async move {
            loop {
                let Ok((conn, _)) = listener.accept().await else {
                    break;
                };
                let allowed = allowed.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(conn, &allowed).await {
                        tracing::debug!(error = %e, "egress proxy connection ended");
                    }
                });
            }
        });
        Ok(Self { port, task })
    }

    /// The proxy variables a sandboxed program gets.
    pub fn env(&self) -> Vec<(String, String)> {
        let url = format!("http://127.0.0.1:{}", self.port);
        [
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "https_proxy",
            "http_proxy",
            "all_proxy",
        ]
        .into_iter()
        .map(|k| (k.to_string(), url.clone()))
        .collect()
    }
}

/// Largest request head the proxy reads.
const MAX_HEAD: usize = 16 * 1024;

async fn handle(mut client: TcpStream, allowed: &[String]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let head_end = loop {
        let mut chunk = [0u8; 2048];
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return refuse(&mut client, "400 Bad Request", "request head too large").await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let rest = buf[head_end..].to_vec();
    let mut line = head.lines().next().unwrap_or_default().split_whitespace();
    let (method, target, version) = (
        line.next().unwrap_or_default(),
        line.next().unwrap_or_default(),
        line.next().unwrap_or("HTTP/1.1"),
    );
    let (host, port, forward_head) = if method.eq_ignore_ascii_case("CONNECT") {
        let Some((h, p)) = target.rsplit_once(':') else {
            return refuse(&mut client, "400 Bad Request", "CONNECT needs host:port").await;
        };
        (
            h.trim_matches(['[', ']']).to_string(),
            p.parse().unwrap_or(443),
            None,
        )
    } else {
        // Plain HTTP through a proxy: `GET http://host[:port]/path HTTP/1.1`.
        let Some(after) = target.strip_prefix("http://") else {
            return refuse(
                &mut client,
                "400 Bad Request",
                "only CONNECT and http:// requests are proxied",
            )
            .await;
        };
        let (authority, path) = match after.find('/') {
            Some(i) => (&after[..i], &after[i..]),
            None => (after, "/"),
        };
        let (h, p) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().unwrap_or(80)),
            None => (authority, 80),
        };
        let rewritten = head.replacen(
            &format!("{method} {target} {version}"),
            &format!("{method} {path} {version}"),
            1,
        );
        (h.to_string(), p, Some(rewritten))
    };
    if !host_allowed(&host, allowed) {
        return refuse(
            &mut client,
            "403 Forbidden",
            &format!("{host} isn't in this source's network list"),
        )
        .await;
    }
    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(e) => return refuse(&mut client, "502 Bad Gateway", &e.to_string()).await,
    };
    match forward_head {
        None => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?
        }
        Some(h) => upstream.write_all(h.as_bytes()).await?,
    }
    upstream.write_all(&rest).await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn refuse(client: &mut TcpStream, status: &str, why: &str) -> std::io::Result<()> {
    let body = format!("oxplow: {why}\n");
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(reply.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_patterns_match_exactly_or_by_subdomain() {
        let p = vec![
            "api.github.com".to_string(),
            "*.githubusercontent.com".to_string(),
        ];
        assert!(host_allowed("api.github.com", &p));
        assert!(host_allowed("API.GitHub.com.", &p));
        assert!(host_allowed("raw.githubusercontent.com", &p));
        assert!(
            !host_allowed("githubusercontent.com", &p),
            "a wildcard is subdomains only"
        );
        assert!(!host_allowed("evil-githubusercontent.com", &p));
        assert!(!host_allowed("github.com", &p));
        assert!(!host_allowed("api.github.com.evil.com", &p));
        assert!(!host_allowed("anything", &[]));
    }

    #[test]
    fn valid_patterns() {
        for ok in [
            "api.github.com",
            "*.githubusercontent.com",
            "localhost",
            "a-b.c1.io",
        ] {
            assert!(valid_host_pattern(ok), "{ok}");
        }
        for bad in [
            "https://api.github.com",
            "api.github.com:443",
            "*",
            "*.",
            "API.github.com",
            "a..b",
            "github",
            "api.github.com/x",
        ] {
            assert!(!valid_host_pattern(bad), "{bad}");
        }
    }

    /// A one-shot HTTP server on localhost answering `ok`.
    async fn origin() -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                tokio::spawn(async move {
                    let mut b = [0u8; 4096];
                    let n = s.read(&mut b).await.unwrap_or(0);
                    let line = String::from_utf8_lossy(&b[..n])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let body = format!("ok {line}");
                    let _ = s
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                });
            }
        });
        port
    }

    async fn roundtrip(proxy: u16, request: String) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", proxy)).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn the_proxy_forwards_declared_hosts_and_refuses_the_rest() {
        let origin = origin().await;
        let proxy = EgressProxy::start(vec!["localhost".into()]).await.unwrap();
        // Plain HTTP: forwarded with the request line in origin form.
        let ok = roundtrip(
            proxy.port,
            format!("GET http://localhost:{origin}/x HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
        .await;
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains("ok GET /x HTTP/1.1"), "{ok}");
        // CONNECT: tunnelled.
        let mut s = TcpStream::connect(("127.0.0.1", proxy.port)).await.unwrap();
        s.write_all(format!("CONNECT localhost:{origin} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut b = [0u8; 64];
        let n = s.read(&mut b).await.unwrap();
        assert!(String::from_utf8_lossy(&b[..n]).starts_with("HTTP/1.1 200 Connection Established"));
        // Anything else: 403, nothing sent upstream.
        let no = roundtrip(
            proxy.port,
            "CONNECT example.com:443 HTTP/1.1\r\n\r\n".to_string(),
        )
        .await;
        assert!(no.starts_with("HTTP/1.1 403"), "{no}");
        assert!(no.contains("example.com isn't in this source's network list"));
        let no = roundtrip(
            proxy.port,
            format!("GET http://127.0.0.1:{origin}/ HTTP/1.1\r\n\r\n"),
        )
        .await;
        assert!(
            no.starts_with("HTTP/1.1 403"),
            "the IP isn't the declared name: {no}"
        );
    }
}
