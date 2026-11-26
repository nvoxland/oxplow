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
use crate::devices::PairedDevice;

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
/// origin, `https` unless it says `http`. Pure.
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

/// Pair this app with the host at `address` using `code`, as a desktop named
/// `name`; keep its token in `secrets` and the host in the home.
pub async fn pair(
    config_dir: &Path,
    secrets: &dyn SecretStore,
    address: &str,
    code: &str,
    name: &str,
) -> Result<RemoteHost, String> {
    let origin = origin_of(address)?;
    let answer = reqwest::Client::new()
        .post(format!("{origin}/host/pair"))
        .json(&serde_json::json!({ "code": code.trim(), "name": name, "kind": "desktop" }))
        .send()
        .await
        .map_err(|e| format!("{origin} did not answer: {e}"))?;
    if !answer.status().is_success() {
        let body: serde_json::Value = answer.json().await.unwrap_or_default();
        return Err(body["error"]
            .as_str()
            .unwrap_or("the host refused the code")
            .to_string());
    }
    #[derive(Deserialize)]
    struct Paired {
        device: PairedDevice,
        token: String,
    }
    let paired: Paired = answer.json().await.map_err(|e| e.to_string())?;
    secrets
        .set(&account(&origin), &paired.token)
        .map_err(|e| format!("could not keep the host's token: {e}"))?;
    let host = RemoteHost {
        origin: origin.clone(),
        device_id: paired.device.id.to_string(),
        device_name: paired.device.name,
    };
    let mut hosts: Vec<RemoteHost> = list(config_dir)
        .into_iter()
        .filter(|h| h.origin != origin)
        .collect();
    hosts.push(host.clone());
    save(config_dir, hosts).map_err(|e| e.to_string())?;
    Ok(host)
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
        let (code, _) = owner.pairing_code().await.unwrap();
        let home = tempfile::tempdir().unwrap();
        let secrets = MemorySecrets::default();

        let wrong = pair(home.path(), &secrets, &url, "WRON-GCOD", "Studio").await;
        assert!(wrong.unwrap_err().contains("wrong"));
        let host = pair(home.path(), &secrets, &url, &code, "Studio")
            .await
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
