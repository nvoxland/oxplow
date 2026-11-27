//! The host's own HTTPS certificate, so other devices need nothing installed
//! but this: a root made once per home (`host-ca.pem`, its key in
//! `host-ca-key.pem`, mode 0600), which a device trusts once, and a
//! certificate for this machine's addresses issued from it at each start.
//!
//! The root may vouch only for private networks (its name constraints: the
//! private and shared IPv4 ranges, IPv6 unique-local and link-local, and
//! names under `local`, `lan`, `home.arpa`, `internal` and `ts.net`), so its
//! key, should it leak, still cannot stand in for a real site. A certificate
//! signed by another CA (`--tls-cert`) takes its place when given.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};

/// How long the root is good: devices trust it once.
const ROOT_YEARS: i64 = 20;
/// How long an issued certificate is good; it is issued again at each start.
const LEAF_DAYS: i64 = 397;
/// The names the root may vouch for, below these.
const PRIVATE_DNS: &[&str] = &["local", "lan", "home.arpa", "internal", "ts.net"];

#[derive(Debug, thiserror::Error)]
pub enum CertsError {
    #[error("the home's certificate files: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot make a certificate: {0}")]
    Make(String),
    #[error("{0} is not on a private network; the host's own certificate covers only those")]
    NotPrivate(String),
}

fn make(e: rcgen::Error) -> CertsError {
    CertsError::Make(e.to_string())
}

pub fn root_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-ca.pem")
}

fn key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-ca-key.pem")
}

fn name_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host-ca-name.txt")
}

/// Whether `ip` is in a range the root may vouch for.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Whether `name` is under a suffix the root may vouch for.
pub fn is_private_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PRIVATE_DNS
        .iter()
        .any(|d| lower == *d || lower.ends_with(&format!(".{d}")))
}

fn root_params(common_name: &str) -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, "Oxplow");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let v4 = |a: [u8; 4], m: [u8; 4]| GeneralSubtree::IpAddress(CidrSubnet::V4(a, m));
    let mut ff = [0u8; 16];
    ff[0] = 0xfe;
    let mut permitted = vec![
        v4([10, 0, 0, 0], [255, 0, 0, 0]),
        v4([172, 16, 0, 0], [255, 240, 0, 0]),
        v4([192, 168, 0, 0], [255, 255, 0, 0]),
        v4([100, 64, 0, 0], [255, 192, 0, 0]),
        v4([169, 254, 0, 0], [255, 255, 0, 0]),
        v4([127, 0, 0, 0], [255, 0, 0, 0]),
        GeneralSubtree::IpAddress(CidrSubnet::V6(
            {
                let mut a = [0u8; 16];
                a[0] = 0xfc;
                a
            },
            ff,
        )),
        GeneralSubtree::IpAddress(CidrSubnet::V6(
            {
                let mut a = [0u8; 16];
                a[0] = 0xfe;
                a[1] = 0x80;
                a
            },
            {
                let mut m = [0u8; 16];
                m[0] = 0xff;
                m[1] = 0xc0;
                m
            },
        )),
    ];
    permitted.extend(
        PRIVATE_DNS
            .iter()
            .map(|d| GeneralSubtree::DnsName((*d).to_string())),
    );
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: permitted,
        excluded_subtrees: Vec::new(),
    });
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(365 * ROOT_YEARS);
    params
}

/// The home's root: made the first time, kept after.
pub struct LocalRoot {
    common_name: String,
    key: KeyPair,
}

impl LocalRoot {
    pub fn load_or_make(config_dir: &Path) -> Result<Self, CertsError> {
        if let (Ok(name), Ok(key)) = (
            std::fs::read_to_string(name_path(config_dir)),
            std::fs::read_to_string(key_path(config_dir)),
        ) {
            if root_path(config_dir).is_file() {
                let key = KeyPair::from_pem(&key).map_err(make)?;
                return Ok(Self {
                    common_name: name.trim().to_string(),
                    key,
                });
            }
        }
        std::fs::create_dir_all(config_dir)?;
        let key = KeyPair::generate().map_err(make)?;
        // A name of its own, so two homes' roots never look alike on a device.
        let common_name = format!(
            "Oxplow on this computer ({})",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let root = root_params(&common_name).self_signed(&key).map_err(make)?;
        write_private(&key_path(config_dir), key.serialize_pem().as_bytes())?;
        std::fs::write(name_path(config_dir), &common_name)?;
        oxplow_config::write_atomic(&root_path(config_dir), root.pem().as_bytes())?;
        tracing::info!(root = %root_path(config_dir).display(), "made the host's own HTTPS root");
        Ok(Self { common_name, key })
    }

    /// A certificate for `ips` and `names` (all on private networks), and
    /// its key, as PEM.
    pub fn issue(&self, ips: &[IpAddr], names: &[String]) -> Result<(String, String), CertsError> {
        let mut sans = Vec::new();
        for ip in ips {
            if !is_private(*ip) {
                return Err(CertsError::NotPrivate(ip.to_string()));
            }
            sans.push(SanType::IpAddress(*ip));
        }
        for name in names {
            if !is_private_name(name) {
                return Err(CertsError::NotPrivate(name.clone()));
            }
            let lower = name.to_ascii_lowercase();
            sans.push(SanType::DnsName(
                lower.try_into().map_err(|e: rcgen::Error| make(e))?,
            ));
        }
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Oxplow");
        params.distinguished_name = dn;
        params.subject_alt_names = sans;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(LEAF_DAYS);
        let key = KeyPair::generate().map_err(make)?;
        let issuer = Issuer::new(root_params(&self.common_name), &self.key);
        let cert = params.signed_by(&key, &issuer).map_err(make)?;
        Ok((cert.pem(), key.serialize_pem()))
    }
}

/// A file only this user may read.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)
}

/// This machine's addresses on private networks, for the Devices window to
/// offer: the ones another device can reach it by.
pub fn private_addresses() -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| is_private(*ip) && !ip.is_loopback())
        .filter(|ip| !matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80))
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

/// The name this machine answers to on its local network: its local
/// hostname with `.local` (macOS's System Settings ▸ General ▸ Sharing ▸
/// Local hostname; elsewhere the hostname), the name a device on the same
/// network may reach it by. None when it has none.
pub fn local_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    let raw = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output();
    #[cfg(not(target_os = "macos"))]
    let raw = std::process::Command::new("hostname").output();
    let out = raw.ok().filter(|o| o.status.success())?;
    local_name_from(&String::from_utf8_lossy(&out.stdout))
}

/// `raw` (a hostname as the system gives it) as a `.local` name: trimmed,
/// lower case, `.local` added to a bare name; none when blank. A name that
/// has a domain already keeps it. Pure.
pub fn local_name_from(raw: &str) -> Option<String> {
    let name = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    Some(if name.contains('.') {
        name
    } else {
        format!("{name}.local")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_is_named_on_the_local_network_as_dot_local() {
        assert_eq!(
            local_name_from("Nathans-MacBook-Pro\n"),
            Some("nathans-macbook-pro.local".into())
        );
        assert_eq!(local_name_from("studio.local"), Some("studio.local".into()));
        assert_eq!(local_name_from("  \n"), None);
        assert!(
            is_private_name(&local_name_from("studio").unwrap()),
            "the host's own certificate covers it"
        );
    }

    #[test]
    fn private_means_the_ranges_a_home_or_a_tailnet_uses() {
        for ip in [
            "192.168.1.20",
            "10.0.0.5",
            "172.20.1.1",
            "100.101.102.103",
            "fd7a:115c::1",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "2001:db8::1"] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }

    /// The root is made once; the certificate it issues names the address
    /// and is for a server; a public address or name is refused.
    #[test]
    fn the_root_is_kept_and_issues_for_private_addresses_only() {
        let home = tempfile::tempdir().unwrap();
        LocalRoot::load_or_make(home.path()).unwrap();
        let first = std::fs::read(root_path(home.path())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_path(home.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the root's key is this user's only");
        }
        let again = LocalRoot::load_or_make(home.path()).unwrap();
        assert_eq!(
            std::fs::read(root_path(home.path())).unwrap(),
            first,
            "made once"
        );
        let (cert, key) = again
            .issue(&["192.168.1.20".parse().unwrap()], &["studio.local".into()])
            .unwrap();
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"), "{cert}");
        assert!(key.contains("PRIVATE KEY"), "{key}");
        assert!(matches!(
            again.issue(&["8.8.8.8".parse().unwrap()], &[]),
            Err(CertsError::NotPrivate(_))
        ));
        assert!(matches!(
            again.issue(&[], &["example.com".into()]),
            Err(CertsError::NotPrivate(_))
        ));
    }

    /// What it issues works: a TLS client that trusts the root alone
    /// reaches the host by its address.
    #[tokio::test]
    async fn a_device_that_trusts_the_root_reaches_the_host() {
        let home = tempfile::tempdir().unwrap();
        let root = LocalRoot::load_or_make(home.path()).unwrap();
        let (cert, key) = root.issue(&["127.0.0.1".parse().unwrap()], &[]).unwrap();
        let (c, k) = (home.path().join("c.pem"), home.path().join("k.pem"));
        std::fs::write(&c, cert).unwrap();
        std::fs::write(&k, key).unwrap();
        let acceptor = crate::tls::acceptor(&c, &k).unwrap();
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = tcp.local_addr().unwrap().port();
        let listener = crate::tls::TlsListener::new(tcp, acceptor).unwrap();
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "secure" }));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let root_pem = std::fs::read(root_path(home.path())).unwrap();
        let client = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(&root_pem).unwrap()])
            .build()
            .unwrap();
        let body = client
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "secure");
    }
}
