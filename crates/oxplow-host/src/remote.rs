//! Hosts on other machines this Mac app has paired with (a headless host on a
//! server: `oxplow host serve`). The app is a paired device there like a
//! tablet: it pairs once with a code the host's owner makes (`oxplow host
//! pair` on the server), keeps the device token in the keychain (never a
//! file an agent can read) and the host in `remote-hosts.json` in the home.
//! A window drives `<origin>/p/<id>` with that token. The app never stops a
//! remote host, and quitting releases nothing there: the work goes on, and the
//! windows come back at the next start.

use std::path::{Path, PathBuf};

use oxplow_ai::secrets::SecretStore;
use serde::{Deserialize, Serialize};

use crate::client::{ClientError, HostClient};

/// A host this app paired with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHost {
    /// `https://server.tailnet.ts.net:7400`.
    pub origin: String,
    /// The id the host gave this Mac at pairing.
    pub device_id: String,
    pub device_name: String,
}

#[derive(Default, Serialize, Deserialize)]
struct RemoteHostsFile {
    #[serde(default)]
    hosts: Vec<RemoteHost>,
}

fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("remote-hosts.json")
}

/// The keychain account of a host's device token.
fn account(origin: &str) -> String {
    format!("oxplow-host:{origin}")
}

/// An address as typed (`server.ts.net:7400`, a trailing slash): its
/// origin, `https` unless it says `http`. Plain `http` only to this machine
/// (an SSH tunnel to `127.0.0.1`): elsewhere the code and the token would
/// pass in clear, and the app's CSP lets a window call no other. Pure.
pub fn origin_of(address: &str) -> Result<String, String> {
    let trimmed = address.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("the host's address is needed".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme)
        .map_err(|e| format!("\"{address}\" is no address: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("\"{address}\" is no web address"));
    }
    let loopback = matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback());
    if url.scheme() == "http" && !loopback {
        return Err(format!(
            "{address} is plain HTTP: give its https:// address (plain HTTP only through a tunnel to 127.0.0.1)"
        ));
    }
    Ok(url.origin().ascii_serialization())
}

pub fn list(config_dir: &Path) -> Vec<RemoteHost> {
    std::fs::read(path(config_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RemoteHostsFile>(&bytes).ok())
        .map(|f| f.hosts)
        .unwrap_or_default()
}

fn save(config_dir: &Path, hosts: Vec<RemoteHost>) -> std::io::Result<()> {
    let bytes =
        serde_json::to_vec_pretty(&RemoteHostsFile { hosts }).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(config_dir)?;
    oxplow_config::write_atomic(&path(config_dir), &bytes)
}

/// Ask the host at `address` to pair this app, as a desktop named `name`:
/// its origin, and the ask (the code for the person to type there, with
/// `oxplow host pair <code>` or its Devices page).
pub async fn ask(address: &str, name: &str) -> Result<(String, crate::client::PairAsk), String> {
    let origin = origin_of(address)?;
    let ask = crate::client::ask_to_pair(&origin, name, "desktop").await?;
    Ok((origin, ask))
}

/// Collect the ask `request` at `origin`: once the person approved it
/// there, keep its token in `secrets` and the host in the home, and answer
/// it; none while it still waits; an error once it ended.
pub async fn collect(
    config_dir: &Path,
    secrets: &dyn SecretStore,
    origin: &str,
    request: &str,
) -> Result<Option<RemoteHost>, String> {
    let Some((device, token)) = crate::client::collect_pairing(origin, request).await? else {
        return Ok(None);
    };
    secrets
        .set(&account(origin), &token)
        .map_err(|e| format!("could not keep the host's token: {e}"))?;
    let host = RemoteHost {
        origin: origin.to_string(),
        device_id: device.id.to_string(),
        device_name: device.name,
    };
    let mut hosts: Vec<RemoteHost> = list(config_dir)
        .into_iter()
        .filter(|h| h.origin != origin)
        .collect();
    hosts.push(host.clone());
    save(config_dir, hosts).map_err(|e| e.to_string())?;
    Ok(Some(host))
}

/// The client for a paired host, with its token from `secrets`.
pub fn client(secrets: &dyn SecretStore, origin: &str) -> Result<HostClient, String> {
    match secrets.get(&account(origin)) {
        Ok(Some(token)) => Ok(HostClient::new(origin, &token)),
        Ok(None) => Err(format!(
            "this Mac is not paired with {origin} any more; connect again"
        )),
        Err(e) => Err(format!("could not read the token for {origin}: {e}")),
    }
}

/// Forget a host: tell it (best effort) and drop the token and the entry.
pub async fn forget(
    config_dir: &Path,
    secrets: &dyn SecretStore,
    origin: &str,
) -> Result<(), String> {
    if let Ok(host) = client(secrets, origin) {
        let _ = host.forget_me().await;
    }
    secrets
        .delete(&account(origin))
        .map_err(|e| format!("could not drop the host's token: {e}"))?;
    let hosts = list(config_dir)
        .into_iter()
        .filter(|h| h.origin != origin)
        .collect();
    save(config_dir, hosts).map_err(|e| e.to_string())
}

impl HostClient {
    /// This device forgets itself on the host.
    pub async fn forget_me(&self) -> Result<(), ClientError> {
        self.delete_me().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::{Fixture, RUNS};
    use oxplow_ai::secrets::MemorySecrets;

    #[test]
    fn an_address_as_typed_is_an_origin() {
        assert_eq!(
            origin_of("server.ts.net:7400/").unwrap(),
            "https://server.ts.net:7400"
        );
        assert_eq!(
            origin_of(" http://127.0.0.1:7400 ").unwrap(),
            "http://127.0.0.1:7400"
        );
        assert!(
            origin_of("http://server.ts.net:7400").is_err(),
            "no plain HTTP off this machine"
        );
        assert!(
            origin_of("http://localhost:7400").is_err(),
            "the CSP allows 127.0.0.1 only"
        );
        assert!(origin_of("").is_err());
        assert!(origin_of("ftp://x").is_err());
    }

    /// The app pairs with a host on another machine, keeps the token in
    /// the keychain (never the file), opens a project there and forgets it.
    #[tokio::test]
    async fn the_app_pairs_with_a_remote_host_and_forgets_it() {
        let fx = Fixture::new(RUNS);
        let url = crate::client::tests::run_host(&fx, "owner").await;
        let owner = HostClient::new(&url, "owner");
        let home = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();

        let (origin, asked) = ask(&url, "Studio").await.unwrap();
        assert_eq!(origin, url);
        let waiting = collect(home.path(), &secrets, &url, &asked.request);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_millis(200), waiting)
                .await
                .ok(),
            None,
            "it waits for the person there"
        );
        assert!(list(home.path()).is_empty());
        owner.approve_pairing(&asked.code).await.unwrap();
        let host = collect(home.path(), &secrets, &url, &asked.request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(host.origin, url);
        assert_eq!(list(home.path()), vec![host.clone()]);
        let file = std::fs::read_to_string(home.path().join("remote-hosts.json")).unwrap();
        let token = secrets.get(&account(&url)).unwrap().unwrap();
        assert!(!file.contains(&token), "the token is in the keychain only");

        let dir = fx.dir.path().join("alpha");
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        let id = fx.host.registry().add(&dir).unwrap().id;
        let remote = client(&secrets, &url).unwrap();
        remote.hold_project(&id).await.unwrap();
        assert_eq!(
            fx.host.holds()[0].device,
            host.device_id,
            "held as this Mac"
        );

        forget(home.path(), &secrets, &url).await.unwrap();
        assert!(list(home.path()).is_empty());
        assert!(fx.host.holds().is_empty(), "its hold went with the device");
        assert!(client(&secrets, &url).is_err());
    }
}
