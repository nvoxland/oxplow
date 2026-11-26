//! HTTPS for other devices. A browser on a tablet needs a secure context for
//! the microphone, the clipboard and more, and a private network's own
//! certificate gives one: `tailscale cert <name>` writes a certificate and
//! key for the machine's tailnet name, `mkcert` one for a LAN name. The host
//! reads the two PEM files at start (`serve --public-bind <addr> --tls-cert
//! <pem> --tls-key <pem>`).
//!
//! Handshakes run in their own tasks, so a client that stalls in one never
//! holds up the others.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

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
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .map_err(|e| TlsError::Cert(cert.display().to_string(), e.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Cert(cert.display().to_string(), e.to_string()))?;
    if chain.is_empty() {
        return Err(TlsError::Cert(
            cert.display().to_string(),
            "no certificate in it".into(),
        ));
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .map_err(|e| TlsError::Key(key.display().to_string(), e.to_string()))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| TlsError::Config(e.to_string()))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// A TCP listener whose connections are TLS, for `axum::serve`.
pub struct TlsListener {
    local: std::net::SocketAddr,
    ready: tokio::sync::mpsc::Receiver<(TlsStream<TcpStream>, std::net::SocketAddr)>,
}

impl TlsListener {
    pub fn new(tcp: TcpListener, acceptor: TlsAcceptor) -> std::io::Result<Self> {
        let local = tcp.local_addr()?;
        let (done, ready) = tokio::sync::mpsc::channel(WAITING);
        tokio::spawn(async move {
            loop {
                let (stream, peer) = match tcp.accept().await {
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
        Ok(Self { local, ready })
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(conn) => conn,
            // The accept loop ended (it never does while the listener
            // lives): no connection will come.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let listener = TlsListener::new(tcp, acceptor).unwrap();
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
