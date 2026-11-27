//! The listener for other devices, as the settings in force say, switched
//! while the host runs.
//!
//! A change is prepared before anything stops: the certificate and the
//! bind. A failure there is the answer and changes nothing. Then the new
//! listener takes over, and the old one's sockets close with their own
//! code (`OTHER_DEVICES_OFF_CLOSE_CODE`) before it is cut (its port freed
//! and every connection that came in on it ended). Moving to the address
//! and port the listener already has frees them first.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use oxplow_host_access::settings::Effective;
use tokio_util::sync::CancellationToken;

use crate::access::{Access, Listener, PublicListener, OTHER_DEVICES_OFF_CLOSE_CODE};
use crate::web::AllowedHosts;

/// What the listener for other devices is to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub addr: SocketAddr,
    /// The names a device may open besides the address: the settings'
    /// names, and this computer's `.local` name with the host's own
    /// certificate.
    pub names: Vec<String>,
    /// A certificate and key of the person's own; none is the home's.
    pub tls: Option<(PathBuf, PathBuf)>,
}

impl Wanted {
    /// The listener `effective` asks for, `local` being this computer's
    /// `.local` name; none while other devices are off. Pure.
    pub fn of(effective: &Effective, local: Option<&str>) -> Option<Self> {
        if !effective.allow_other_devices {
            return None;
        }
        let ip = effective.address.parse().ok()?;
        let tls = effective.tls_cert.clone().zip(effective.tls_key.clone());
        let names = if tls.is_none() {
            oxplow_host_access::network::with_local_name(&effective.names, local)
        } else {
            effective.names.clone()
        };
        Some(Self {
            addr: SocketAddr::new(ip, effective.port.port),
            names,
            tls,
        })
    }
}

/// The names besides loopback any listener answers to: the address and
/// names of the listener for other devices, else the names the settings in
/// force give (a `--allow-host` name for a proxy in front of loopback). Pure.
pub fn allowed_names(effective: &Effective, wanted: Option<&Wanted>) -> Vec<String> {
    match wanted {
        Some(w) => std::iter::once(w.addr.ip().to_string())
            .chain(w.names.iter().cloned())
            .collect(),
        None => effective.names.clone(),
    }
}

struct Running {
    wanted: Wanted,
    cut: CancellationToken,
}

/// The listener for other devices of one host.
pub struct PublicServer {
    config_dir: PathBuf,
    access: Arc<Access>,
    root_pem: Arc<Mutex<Option<String>>>,
    allowed: AllowedHosts,
    /// The app every connection of this listener is served by.
    make_app: Box<dyn Fn() -> Router + Send + Sync>,
    running: tokio::sync::Mutex<Option<Running>>,
}

impl PublicServer {
    pub fn new(
        config_dir: &Path,
        access: Arc<Access>,
        root_pem: Arc<Mutex<Option<String>>>,
        allowed: AllowedHosts,
        make_app: Box<dyn Fn() -> Router + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config_dir: config_dir.to_path_buf(),
            access,
            root_pem,
            allowed,
            make_app,
            running: tokio::sync::Mutex::new(None),
        })
    }

    /// Make the listener what `effective` says. Nothing happens when it
    /// already is. A certificate or bind that fails is the answer and the
    /// listener stays as it was (one already on that address and port is
    /// off then, and says why).
    pub async fn apply(&self, effective: &Effective) -> Result<(), String> {
        let local = oxplow_host_access::network::local_name();
        let wanted = Wanted::of(effective, local.as_deref());
        let mut running = self.running.lock().await;
        if running.as_ref().map(|r| &r.wanted) == wanted.as_ref() {
            self.allowed.set(allowed_names(effective, wanted.as_ref()));
            return Ok(());
        }
        let Some(wanted) = wanted else {
            if let Some(old) = running.take() {
                self.cut(old).await;
            }
            self.set_public(PublicListener::Off);
            self.allowed.set(allowed_names(effective, None));
            tracing::info!("other devices are off");
            return Ok(());
        };
        let (acceptor, root) = self.acceptor(&wanted)?;
        let same_place = running
            .as_ref()
            .is_some_and(|r| r.wanted.addr == wanted.addr);
        if same_place {
            if let Some(old) = running.take() {
                self.cut(old).await;
            }
        }
        let tcp = match bind(wanted.addr, same_place).await {
            Ok(tcp) => tcp,
            Err(e) => {
                let error = format!("bind {} failed: {e}", wanted.addr);
                if same_place {
                    self.set_public(PublicListener::Failed {
                        error: error.clone(),
                    });
                    self.allowed.set(allowed_names(effective, None));
                }
                return Err(error);
            }
        };
        if let Some(old) = running.take() {
            self.cut(old).await;
        }
        let at = tcp.local_addr().map_err(|e| e.to_string())?;
        let cut = CancellationToken::new();
        let listener =
            crate::tls::TlsListener::new(tcp, acceptor, cut.clone()).map_err(|e| e.to_string())?;
        let app = (self.make_app)();
        tokio::spawn(async move { axum::serve(listener, app).await });
        *self.root_pem.lock().unwrap_or_else(|e| e.into_inner()) = root;
        self.allowed.set(allowed_names(effective, Some(&wanted)));
        self.set_public(PublicListener::Listening {
            url: format!("https://{at}"),
            names: wanted.names.clone(),
        });
        tracing::info!(%at, "listening for other devices");
        *running = Some(Running { wanted, cut });
        Ok(())
    }

    /// The acceptor for `wanted`, and the home's root when the certificate
    /// is one of its own.
    fn acceptor(
        &self,
        wanted: &Wanted,
    ) -> Result<(tokio_rustls::TlsAcceptor, Option<String>), String> {
        match &wanted.tls {
            Some((cert, key)) => crate::tls::acceptor(cert, key)
                .map(|a| (a, None))
                .map_err(|e| e.to_string()),
            None => issued_acceptor(&self.config_dir, wanted.addr, &wanted.names),
        }
    }

    /// Close the old listener's sockets with their code, give them a
    /// moment to say so, then cut it.
    async fn cut(&self, old: Running) {
        self.access
            .tunnels
            .close_listener(Listener::Public, OTHER_DEVICES_OFF_CLOSE_CODE);
        tokio::time::sleep(Duration::from_millis(200)).await;
        old.cut.cancel();
    }

    fn set_public(&self, state: PublicListener) {
        *self.access.public.lock().unwrap_or_else(|e| e.into_inner()) = state;
        self.access.changed();
    }
}

/// Bind `addr`; one just freed (`retry`) gets a moment, as its accept loop
/// lets go.
async fn bind(addr: SocketAddr, retry: bool) -> std::io::Result<tokio::net::TcpListener> {
    let mut tries = if retry { 20 } else { 1 };
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(tcp) => return Ok(tcp),
            Err(e) if tries > 1 && e.kind() == std::io::ErrorKind::AddrInUse => {
                tries -= 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// TLS for the listener at `addr`, from the home's own root: a certificate
/// for that address (every private address of this machine when it is
/// unspecified) and `names`, issued now. The root's PEM comes too, for
/// devices to download (`/host/ca.pem`).
fn issued_acceptor(
    config_dir: &Path,
    addr: SocketAddr,
    names: &[String],
) -> Result<(tokio_rustls::TlsAcceptor, Option<String>), String> {
    let root = crate::certs::LocalRoot::load_or_make(config_dir).map_err(|e| e.to_string())?;
    let ips = if addr.ip().is_unspecified() {
        oxplow_host_access::network::private_addresses()
    } else {
        vec![addr.ip()]
    };
    let (cert, key) = root.issue(&ips, names).map_err(|e| {
        format!("{e}; give a certificate of your own (--tls-cert, --tls-key) for that")
    })?;
    let acceptor = crate::tls::acceptor_pem(
        cert.as_bytes(),
        key.as_bytes(),
        "the issued certificate",
        "its key",
    )
    .map_err(|e| e.to_string())?;
    let root_pem = std::fs::read_to_string(oxplow_host_access::root_path(config_dir))
        .map_err(|e| e.to_string())?;
    Ok((acceptor, Some(root_pem)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_host_access::settings::{DevicePort, PortFrom};

    fn on(address: &str, port: u16) -> Effective {
        Effective {
            allow_other_devices: true,
            address: address.into(),
            port: DevicePort {
                port,
                from: PortFrom::Flag,
            },
            names: vec!["mac.ts.net".into()],
            tls_cert: None,
            tls_key: None,
            overridden: vec![],
        }
    }

    #[test]
    fn the_listener_wanted_follows_the_settings_in_force() {
        let off = Effective {
            allow_other_devices: false,
            ..on("192.168.1.20", 7400)
        };
        assert_eq!(Wanted::of(&off, Some("studio.local")), None);
        assert_eq!(
            allowed_names(&off, None),
            ["mac.ts.net"],
            "a proxy's name stays"
        );

        let wanted = Wanted::of(&on("192.168.1.20", 7400), Some("studio.local")).unwrap();
        assert_eq!(wanted.addr, "192.168.1.20:7400".parse().unwrap());
        assert_eq!(wanted.names, ["mac.ts.net", "studio.local"]);
        assert_eq!(
            allowed_names(&on("192.168.1.20", 7400), Some(&wanted)),
            ["192.168.1.20", "mac.ts.net", "studio.local"]
        );
        let own = Effective {
            tls_cert: Some("/c.pem".into()),
            tls_key: Some("/k.pem".into()),
            ..on("192.168.1.20", 7400)
        };
        assert_eq!(
            Wanted::of(&own, Some("studio.local")).unwrap().names,
            ["mac.ts.net"],
            "a certificate of your own names what it names"
        );
    }

    fn server(fx: &crate::host::tests::Fixture) -> (Arc<PublicServer>, Arc<Access>, AllowedHosts) {
        let access = Access::new(
            "owner".into(),
            fx.db.clone(),
            Default::default(),
            Duration::from_millis(10),
        );
        let allowed = AllowedHosts::default();
        let home = fx.dir.path().join("home");
        let server = PublicServer::new(
            &home,
            access.clone(),
            Default::default(),
            allowed.clone(),
            Box::new(|| Router::new().route("/", axum::routing::get(|| async { "here" }))),
        );
        (server, access, allowed)
    }

    fn public(access: &Access) -> PublicListener {
        access.public.lock().unwrap().clone()
    }

    async fn reach(url: &str) -> Result<String, reqwest::Error> {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(url)
            .send()
            .await?
            .text()
            .await
    }

    /// Turned on, the listener answers at once; moved, the old place stops;
    /// turned off, nothing answers and an open socket there is closed with
    /// its own code.
    #[tokio::test]
    async fn other_devices_turn_on_move_and_turn_off_while_the_host_runs() {
        let fx = crate::host::tests::Fixture::new(crate::host::tests::RUNS);
        let (server, access, allowed) = server(&fx);

        server.apply(&on("127.0.0.1", 0)).await.unwrap();
        let PublicListener::Listening { url: first, .. } = public(&access) else {
            panic!("not listening: {:?}", public(&access));
        };
        assert_eq!(reach(&format!("{first}/")).await.unwrap(), "here");
        assert!(allowed.get().contains(&"127.0.0.1".to_string()));

        // The same settings change nothing.
        server.apply(&on("127.0.0.1", 0)).await.unwrap();
        assert!(matches!(public(&access), PublicListener::Listening { url, .. } if url == first));

        let socket = access.open_tunnel(
            crate::access::Connected {
                device: "dev-tab".into(),
                name: "Tab".into(),
                kind: None,
            },
            Listener::Public,
        );
        let off = Effective {
            allow_other_devices: false,
            ..on("127.0.0.1", 0)
        };
        server.apply(&off).await.unwrap();
        assert_eq!(public(&access), PublicListener::Off);
        assert_eq!(*socket.closed.borrow(), Some(OTHER_DEVICES_OFF_CLOSE_CODE));
        assert!(
            reach(&format!("{first}/")).await.is_err(),
            "nothing answers there"
        );
        assert_eq!(*allowed.get(), ["mac.ts.net"]);
    }

    /// A certificate that cannot be read is the answer, and the listener
    /// stays as it was.
    #[tokio::test]
    async fn a_change_that_cannot_work_changes_nothing() {
        let fx = crate::host::tests::Fixture::new(crate::host::tests::RUNS);
        let (server, access, _) = server(&fx);
        server.apply(&on("127.0.0.1", 0)).await.unwrap();
        let before = public(&access);
        let PublicListener::Listening { url, .. } = &before else {
            panic!("not listening");
        };
        let broken = Effective {
            tls_cert: Some("/nowhere/c.pem".into()),
            tls_key: Some("/nowhere/k.pem".into()),
            ..on("127.0.0.1", 0)
        };
        let err = server.apply(&broken).await.unwrap_err();
        assert!(err.contains("/nowhere/c.pem"), "{err}");
        assert_eq!(public(&access), before);
        assert_eq!(reach(&format!("{url}/")).await.unwrap(), "here");
    }
}
