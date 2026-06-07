//! LSP session manager.
//!
//! Caches one `LspProxy` per `(stream, language)` so JSON-RPC requests
//! issued by the renderer or MCP tools don't pay the spawn-and-init
//! cost on every call. Initialization is lazy: the first request for
//! a given pair spawns the language server and runs the LSP
//! `initialize` handshake.
//!
//! Session lookup uses the `LspServerConfig` from `oxplow.yaml` to
//! find the command for a given language; if no config is present
//! the call returns an error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

use oxplow_config::{LspServerConfig, OxplowConfig};
use oxplow_lsp::{LspError, LspProxy, SpawnConfig};

#[derive(Debug, Error)]
pub enum LspSessionError {
    #[error("no language server configured for language `{0}`")]
    NoConfig(String),
    #[error("lsp: {0}")]
    Lsp(#[from] LspError),
    #[error("lsp not initialized for language `{0}`")]
    NotInitialized(String),
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct SessionKey {
    stream_id: String,
    language: String,
}

/// Runtime registry of language servers installed via the Mason
/// installer. These layer on top of `oxplow.yaml`'s `lspServers` —
/// installer-installed servers are picked up only when no yaml entry
/// matches the language.
#[derive(Clone, Default)]
pub struct InstalledServers {
    inner: Arc<std::sync::RwLock<Vec<LspServerConfig>>>,
}

impl InstalledServers {
    pub fn register(&self, cfg: LspServerConfig) {
        if let Ok(mut g) = self.inner.write() {
            g.retain(|c| c.language_id != cfg.language_id);
            g.push(cfg);
        }
    }

    pub fn list(&self) -> Vec<LspServerConfig> {
        self.inner.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn find(&self, language: &str) -> Option<LspServerConfig> {
        self.inner
            .read()
            .ok()?
            .iter()
            .find(|s| s.language_id == language)
            .cloned()
    }
}

#[derive(Clone)]
pub struct LspSessionManager {
    config: Arc<std::sync::RwLock<OxplowConfig>>,
    installed: InstalledServers,
    sessions: Arc<Mutex<HashMap<SessionKey, Arc<LspProxy>>>>,
}

impl LspSessionManager {
    pub fn new(config: Arc<std::sync::RwLock<OxplowConfig>>) -> Self {
        Self {
            config,
            installed: InstalledServers::default(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn installed_servers(&self) -> &InstalledServers {
        &self.installed
    }

    fn find_server_config(&self, language: &str) -> Option<LspServerConfig> {
        if let Ok(cfg) = self.config.read() {
            if let Some(s) = cfg.lsp_servers.iter().find(|s| s.language_id == language) {
                return Some(s.clone());
            }
        }
        self.installed.find(language)
    }

    /// Get or spawn the LspProxy for `(stream_id, language)`.
    pub async fn ensure(
        &self,
        stream_id: &str,
        language: &str,
        cwd: PathBuf,
    ) -> Result<Arc<LspProxy>, LspSessionError> {
        let key = SessionKey {
            stream_id: stream_id.to_string(),
            language: language.to_string(),
        };
        {
            let map = self.sessions.lock().await;
            if let Some(p) = map.get(&key) {
                return Ok(p.clone());
            }
        }
        let server_config = self
            .find_server_config(language)
            .ok_or_else(|| LspSessionError::NoConfig(language.to_string()))?;

        let proxy = LspProxy::spawn(SpawnConfig {
            command: server_config.command,
            args: server_config.args,
            cwd: Some(cwd.clone()),
        })?;

        // Run the LSP `initialize` handshake. We don't carry full
        // capabilities — minimum is enough to satisfy
        // typescript-language-server.
        let init = proxy
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": format!("file://{}", cwd.display()),
                    "capabilities": {},
                    "workspaceFolders": [{
                        "uri": format!("file://{}", cwd.display()),
                        "name": "oxplow",
                    }],
                }),
            )
            .await?;
        info!(?language, "lsp initialized");
        proxy.notify("initialized", json!({})).await?;
        let _ = init;

        let arc = Arc::new(proxy);
        let mut map = self.sessions.lock().await;
        map.insert(key, arc.clone());
        Ok(arc)
    }

    /// Tear down all sessions for a stream (e.g. on stream delete).
    /// Best-effort; we don't shutdown LSPs cleanly here, just drop.
    pub async fn drop_for_stream(&self, stream_id: &str) {
        let mut map = self.sessions.lock().await;
        let keys: Vec<_> = map
            .keys()
            .filter(|k| k.stream_id == stream_id)
            .cloned()
            .collect();
        for key in keys {
            if map.remove(&key).is_some() {
                warn!(stream_id, language = key.language, "lsp session dropped");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;

    fn empty_config() -> Arc<std::sync::RwLock<OxplowConfig>> {
        Arc::new(std::sync::RwLock::new(OxplowConfig {
            agents: vec![AgentKind::Claude],
            project_name: "p".into(),
            lsp_servers: vec![],
            agent_prompt_append: String::new(),
            snapshot_retention_days: 7,
            generated: vec![],
            snapshot_max_file_bytes: 0,
            inject_session_context: true,
            collection: Default::default(),
        }))
    }

    #[tokio::test]
    async fn ensure_returns_no_config_when_language_unknown() {
        let mgr = LspSessionManager::new(empty_config());
        let err = mgr
            .ensure("s-1", "tsx", std::env::temp_dir())
            .await
            .err()
            .expect("should error");
        assert!(matches!(err, LspSessionError::NoConfig(lang) if lang == "tsx"));
    }

    #[test]
    fn installed_servers_override_take_effect() {
        let mgr = LspSessionManager::new(empty_config());
        mgr.installed_servers().register(LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "/tmp/fake/rust-analyzer".into(),
            args: vec![],
        });
        let cfg = mgr.find_server_config("rust").expect("registered server");
        assert_eq!(cfg.command, "/tmp/fake/rust-analyzer");
        assert!(mgr.find_server_config("python").is_none());
    }

    #[test]
    fn yaml_config_wins_over_installed_for_same_language() {
        let cfg = empty_config();
        cfg.write().unwrap().lsp_servers.push(LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "yaml-rust-analyzer".into(),
            args: vec![],
        });
        let mgr = LspSessionManager::new(cfg);
        mgr.installed_servers().register(LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "installed-rust-analyzer".into(),
            args: vec![],
        });
        assert_eq!(
            mgr.find_server_config("rust").unwrap().command,
            "yaml-rust-analyzer"
        );
    }
}
