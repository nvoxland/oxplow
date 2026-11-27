//! HTTPS for other devices. A browser on a tablet needs a secure context for
//! the microphone, the clipboard and more, and a private network's own
//! certificate gives one: `tailscale cert <name>` writes a certificate and
//! key for the machine's tailnet name, `mkcert` one for a LAN name. The host
//! reads the two PEM files at start (`serve --public-bind <addr> --tls-cert
//! <pem> --tls-key <pem>`).
//!
//! Handshakes run in their own tasks, so a client that stalls in one never
//! holds up the others.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

/// How long a client gets to finish its handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Finished handshakes waiting for the server to take them.
const WAITING: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot read the certificate {0}: {1}")]
    Cert(String, String),
    #[error("cannot read the key {0}: {1}")]
    Key(String, String),
    #[error("the certificate and key do not make a TLS setup: {0}")]
    Config(String),
}

/// The acceptor for the certificate chain and key in these PEM files.
pub fn acceptor(cert: &Path, key: &Path) -> Result<TlsAcceptor, TlsError> {
    let cert_pem = std::fs::read(cert)
        .map_err(|e| TlsError::Cert(cert.display().to_string(), e.to_string()))?;
    let key_pem =
        std::fs::read(key).map_err(|e| TlsError::Key(key.display().to_string(), e.to_string()))?;
    acceptor_pem(
        &cert_pem,
        &key_pem,
        &cert.display().to_string(),
        &key.display().to_string(),
    )
}

/// The acceptor for a certificate chain and key given as PEM (`what_cert`
/// and `what_key` name them in an error).
pub fn acceptor_pem(
    cert_pem: &[u8],
    key_pem: &[u8],
    what_cert: &str,
    what_key: &str,
) -> Result<TlsAcceptor, TlsError> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Cert(what_cert.to_string(), e.to_string()))?;
    if chain.is_empty() {
        return Err(TlsError::Cert(
            what_cert.to_string(),
            "no certificate in it".into(),
        ));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|e| TlsError::Key(what_key.to_string(), e.to_string()))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| TlsError::Config(e.to_string()))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// A TCP listener whose connections are TLS, for `axum::serve`. Its `cut`
/// stops it: the accept loop ends (the port is free again) and every
/// connection that came in on it ends at its next read or write, an open
/// socket included, so turning other devices off leaves nothing open.
pub struct TlsListener {
    local: std::net::SocketAddr,
    ready: tokio::sync::mpsc::Receiver<(TlsStream<TcpStream>, std::net::SocketAddr)>,
    cut: CancellationToken,
}

impl TlsListener {
    pub fn new(
        tcp: TcpListener,
        acceptor: TlsAcceptor,
        cut: CancellationToken,
    ) -> std::io::Result<Self> {
        let local = tcp.local_addr()?;
        let (done, ready) = tokio::sync::mpsc::channel(WAITING);
        let stop = cut.clone();
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stop.cancelled() => return,
                    accepted = tcp.accept() => accepted,
                };
                let (stream, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let acceptor = acceptor.clone();
                let handed = done.clone();
                tokio::spawn(async move {
                    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                        Ok(Ok(tls)) => {
                            let _ = handed.send((tls, peer)).await;
                        }
                        Ok(Err(e)) => tracing::debug!(%peer, error = %e, "a TLS handshake failed"),
                        Err(_) => tracing::debug!(%peer, "a TLS handshake took too long"),
                    }
                });
                if done.is_closed() {
                    return;
                }
            }
        });
        Ok(Self { local, ready, cut })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = Cuttable<TlsStream<TcpStream>>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let conn = tokio::select! {
            conn = self.ready.recv() => conn,
            _ = self.cut.cancelled() => None,
        };
        match conn {
            Some((io, peer)) => (Cuttable::new(io, &self.cut), peer),
            // Cut (or the accept loop ended): no connection will come.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// A connection that ends when its listener is cut: its next read or write
/// fails, which ends whatever it carries.
pub struct Cuttable<S> {
    inner: S,
    cut: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl<S> Cuttable<S> {
    fn new(inner: S, cut: &CancellationToken) -> Self {
        Self {
            inner,
            cut: Box::pin(cut.clone().cancelled_owned()),
        }
    }

    /// Whether the listener was cut (registering the waker, so a task
    /// waiting on this connection wakes when it is).
    fn is_cut(&mut self, cx: &mut Context<'_>) -> bool {
        self.cut.as_mut().poll(cx).is_ready()
    }
}

fn cut_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "other devices were turned off here",
    )
}

impl<S: AsyncRead + Unpin> AsyncRead for Cuttable<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Cuttable<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn testdata(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name)
    }

    #[tokio::test]
    async fn a_device_reaches_the_host_over_https() {
        let acceptor = acceptor(
            &testdata("localhost-cert.pem"),
            &testdata("localhost-key.pem"),
        )
        .unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = tcp.local_addr().unwrap().port();
        let listener = TlsListener::new(tcp, acceptor, CancellationToken::new()).unwrap();
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "secure" }));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        // Trust is the browser's (a tailnet or mkcert certificate); the
        // fixture is self-signed for a century, which macOS's verifier
        // refuses, so this checks only that the host speaks TLS.
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        // A plain-text client that stalls does not hold up the next one.
        let _stalled = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let body = client
            .get(format!("https://localhost:{port}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "secure");
    }

    /// Other devices turned off: the listener stops at once, its port is
    /// free again, and a connection that came in on it is cut, not left
    /// open for its next request.
    #[tokio::test]
    async fn a_cut_listener_frees_its_port_and_ends_its_connections() {
        let acceptor = acceptor(
            &testdata("localhost-cert.pem"),
            &testdata("localhost-key.pem"),
        )
        .unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let cut = tokio_util::sync::CancellationToken::new();
        let listener = TlsListener::new(tcp, acceptor, cut.clone()).unwrap();
        // An answer that never ends, as an open socket's.
        let endless = || async {
            let chunks = futures::stream::once(async { Ok::<_, std::io::Error>("open") })
                .chain(futures::stream::pending());
            axum::body::Body::from_stream(chunks)
        };
        let app = axum::Router::new().route("/", axum::routing::get(endless));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let url = format!("https://localhost:{}/", addr.port());
        let mut open = client.get(&url).send().await.unwrap();
        assert_eq!(open.chunk().await.unwrap().unwrap(), "open");

        cut.cancel();
        let ended = tokio::time::timeout(Duration::from_secs(5), open.chunk()).await;
        assert!(
            matches!(ended, Ok(Err(_)) | Ok(Ok(None))),
            "the open answer is cut: {ended:?}"
        );
        let started = std::time::Instant::now();
        let freed = loop {
            match TcpListener::bind(addr).await {
                Ok(again) => break again,
                Err(_) if started.elapsed() < Duration::from_secs(5) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => panic!("the port stayed taken: {e}"),
            }
        };
        drop(freed);
    }

    #[test]
    fn a_missing_or_empty_certificate_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let key = testdata("localhost-key.pem");
        let missing = acceptor(&dir.path().join("nope.pem"), &key).err().unwrap();
        assert!(missing.to_string().contains("nope.pem"), "{missing}");
        let none = acceptor(&empty, &key).err().unwrap();
        assert!(none.to_string().contains("no certificate"), "{none}");
    }
}
