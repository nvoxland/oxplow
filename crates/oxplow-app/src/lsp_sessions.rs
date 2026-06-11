//! LSP session manager.
//!
//! Owns every language-server process: one `LspProxy` per
//! `(stream, language)`, shared by the renderer (via the `lsp_request`
//! / `lsp_notify` RPCs) and the MCP tools. Initialization is lazy:
//! the first request for a pair spawns the server and runs the LSP
//! `initialize` handshake with real client capabilities.
//!
//! Beyond caching, the manager:
//!   - pumps server-originated messages: notifications are re-emitted
//!     as [`LspSessionEvent::ServerNotification`] on a broadcast the
//!     transports forward to the renderer; server→client *requests*
//!     (`workspace/configuration`, `window/workDoneProgress/create`,
//!     …) are auto-answered so servers like rust-analyzer don't stall;
//!   - mirrors open documents (`didOpen`/`didChange`/`didClose`
//!     full-text sync) so a crashed or restarted server can be
//!     respawned with every open buffer replayed;
//!   - surfaces lifecycle transitions as
//!     [`LspSessionEvent::SessionStatus`].
//!
//! Session lookup uses `oxplow.yaml`'s `lsp.servers` first, then the
//! Mason-installed registry; if neither matches, the error message is
//! self-describing (suggests the Mason package to install).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use thiserror::Error;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

use oxplow_config::{LspServerConfig, OxplowConfig};
use oxplow_lsp::{LspError, LspProxy, ServerEvent, SpawnConfig};

#[derive(Debug, Error)]
pub enum LspSessionError {
    #[error("{}", no_config_message(.0))]
    NoConfig(String),
    #[error("lsp: {0}")]
    Lsp(#[from] LspError),
    #[error("lsp not initialized for language `{0}`")]
    NotInitialized(String),
}

/// Hand-curated language-id → Mason package suggestions, mirrored by
/// `apps/desktop/src/lspSuggestions.ts` (keep the two in sync). Used
/// to make `NoConfig` errors actionable for both agents and the UI.
pub fn mason_suggestion(language: &str) -> Option<&'static str> {
    match language {
        "rust" => Some("rust-analyzer"),
        "go" => Some("gopls"),
        "typescript" | "javascript" | "typescriptreact" | "javascriptreact" => {
            Some("typescript-language-server")
        }
        "python" => Some("pyright"),
        "lua" => Some("lua-language-server"),
        "c" | "cpp" => Some("clangd"),
        "json" => Some("json-lsp"),
        "yaml" => Some("yaml-language-server"),
        "html" => Some("html-lsp"),
        "css" => Some("css-lsp"),
        "bash" | "shell" => Some("bash-language-server"),
        "ruby" => Some("ruby-lsp"),
        "zig" => Some("zls"),
        _ => None,
    }
}

fn no_config_message(language: &str) -> String {
    match mason_suggestion(language) {
        Some(pkg) => format!(
            "no language server configured for `{language}` — install one via \
             lsp_install_server (suggested Mason package: \"{pkg}\") or add an \
             lsp.servers entry to oxplow.yaml"
        ),
        None => format!(
            "no language server configured for `{language}` — install one via \
             lsp_install_server (see mason-registry for package names) or add \
             an lsp.servers entry to oxplow.yaml"
        ),
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
struct SessionKey {
    stream_id: String,
    language: String,
}

/// Lifecycle + server-originated traffic, broadcast to the renderer
/// over the `lsp:event` channel (Tauri emit / daemon `/events` frame).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LspSessionEvent {
    /// A notification the server sent (publishDiagnostics,
    /// window/showMessage, $/progress, …). Forwarded verbatim.
    ServerNotification {
        stream_id: String,
        language: String,
        method: String,
        params: Value,
    },
    /// Session lifecycle transition.
    SessionStatus {
        stream_id: String,
        language: String,
        status: LspSessionStatus,
        message: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LspSessionStatus {
    Ready,
    Crashed,
    Restarted,
    Stopped,
}

/// Where a server config came from, for the settings UI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LspServerSource {
    Yaml,
    Installed,
}

/// One row of `list_servers()` — everything the settings page and the
/// agent need to reason about a configured server.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LspServerListing {
    pub language_id: String,
    pub extensions: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
    pub source: LspServerSource,
    /// Mason package name when `source == Installed`.
    pub package_name: Option<String>,
    /// Installed package version when `source == Installed`.
    pub version: Option<String>,
    pub binary_exists: bool,
    /// Stream ids with a live session for this language.
    pub running_streams: Vec<String>,
    /// `completionProvider.triggerCharacters` from a running session's
    /// server capabilities, when one exists.
    pub completion_trigger_characters: Option<Vec<String>>,
}

/// Runtime registry of language servers installed via the Mason
/// installer. These layer on top of `oxplow.yaml`'s `lspServers` —
/// installer-installed servers are picked up only when no yaml entry
/// matches the language.
#[derive(Clone, Default)]
pub struct InstalledServers {
    inner: Arc<std::sync::RwLock<Vec<InstalledServerEntry>>>,
}

/// An installed server registration: the spawn config plus the Mason
/// package identity it came from (needed for remove + the settings UI).
#[derive(Clone, Debug)]
pub struct InstalledServerEntry {
    pub config: LspServerConfig,
    pub package: String,
    pub version: String,
}

impl InstalledServers {
    pub fn register(&self, cfg: LspServerConfig, package: &str, version: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.retain(|c| c.config.language_id != cfg.language_id);
            g.push(InstalledServerEntry {
                config: cfg,
                package: package.to_string(),
                version: version.to_string(),
            });
        }
    }

    /// Drop every registration that came from `package`.
    pub fn unregister_package(&self, package: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.retain(|c| c.package != package);
        }
    }

    pub fn list(&self) -> Vec<InstalledServerEntry> {
        self.inner.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn find(&self, language: &str) -> Option<LspServerConfig> {
        self.inner
            .read()
            .ok()?
            .iter()
            .find(|s| s.config.language_id == language)
            .map(|s| s.config.clone())
    }
}

/// Mirrored open document, used to replay `didOpen` after a server
/// crash or restart. Full-text sync only.
#[derive(Clone, Debug)]
struct MirrorDoc {
    language_id: String,
    version: i64,
    text: String,
}

struct SessionEntry {
    proxy: Arc<LspProxy>,
    server_capabilities: Value,
    generation: u64,
}

#[derive(Clone)]
pub struct LspSessionManager {
    config: Arc<std::sync::RwLock<OxplowConfig>>,
    installed: InstalledServers,
    sessions: Arc<Mutex<HashMap<SessionKey, SessionEntry>>>,
    docs: Arc<std::sync::Mutex<HashMap<SessionKey, HashMap<String, MirrorDoc>>>>,
    events: broadcast::Sender<LspSessionEvent>,
    next_generation: Arc<std::sync::atomic::AtomicU64>,
}

impl LspSessionManager {
    pub fn new(config: Arc<std::sync::RwLock<OxplowConfig>>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            config,
            installed: InstalledServers::default(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            docs: Arc::new(std::sync::Mutex::new(HashMap::new())),
            events,
            next_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    pub fn installed_servers(&self) -> &InstalledServers {
        &self.installed
    }

    /// Subscribe to session lifecycle + server notification events.
    pub fn subscribe(&self) -> broadcast::Receiver<LspSessionEvent> {
        self.events.subscribe()
    }

    /// Test seam: inject an event onto the broadcast as if a session
    /// pump produced it (transport tests don't want a real server).
    #[doc(hidden)]
    pub fn emit_event_for_tests(&self, event: LspSessionEvent) {
        let _ = self.events.send(event);
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
            if let Some(s) = map.get(&key) {
                return Ok(s.proxy.clone());
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

        let init = proxy
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": format!("file://{}", cwd.display()),
                    "capabilities": client_capabilities(),
                    "workspaceFolders": [{
                        "uri": format!("file://{}", cwd.display()),
                        "name": "oxplow",
                    }],
                }),
            )
            .await?;
        info!(?language, "lsp initialized");
        proxy.notify("initialized", json!({})).await?;
        let server_capabilities = init.get("capabilities").cloned().unwrap_or(Value::Null);

        let arc = Arc::new(proxy);
        let generation = self
            .next_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.spawn_pump(key.clone(), &arc, generation);

        // Replay any mirrored documents (crash/restart recovery; no-op
        // on first spawn).
        let mirrored: Vec<(String, MirrorDoc)> = self
            .docs
            .lock()
            .expect("docs mutex")
            .get(&key)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (uri, doc) in mirrored {
            let _ = arc
                .notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": doc.language_id,
                            "version": doc.version,
                            "text": doc.text,
                        }
                    }),
                )
                .await;
        }

        let mut map = self.sessions.lock().await;
        map.insert(
            key.clone(),
            SessionEntry {
                proxy: arc.clone(),
                server_capabilities,
                generation,
            },
        );
        let _ = self.events.send(LspSessionEvent::SessionStatus {
            stream_id: key.stream_id,
            language: key.language,
            status: LspSessionStatus::Ready,
            message: None,
        });
        Ok(arc)
    }

    /// Forward server-originated traffic for one session: notifications
    /// become broadcast events; server→client requests are auto-answered
    /// so the server never stalls waiting on a client we don't fully
    /// implement; `Closed` tears the session down (unless it was already
    /// intentionally replaced — generation mismatch).
    fn spawn_pump(&self, key: SessionKey, proxy: &Arc<LspProxy>, generation: u64) {
        let mut rx = proxy.events();
        let weak = Arc::downgrade(proxy);
        let sessions = self.sessions.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ServerEvent::Notification { method, params }) => {
                        let _ = events.send(LspSessionEvent::ServerNotification {
                            stream_id: key.stream_id.clone(),
                            language: key.language.clone(),
                            method,
                            params,
                        });
                    }
                    Ok(ServerEvent::Request { id, method, params }) => {
                        let Some(proxy) = weak.upgrade() else { break };
                        let result = auto_answer(&method, &params);
                        if let Err(e) = proxy.respond(id, result).await {
                            warn!(?e, method, "lsp auto-answer failed");
                        }
                    }
                    Ok(ServerEvent::Closed) => {
                        let mut map = sessions.lock().await;
                        let still_current = map
                            .get(&key)
                            .map(|s| s.generation == generation)
                            .unwrap_or(false);
                        if still_current {
                            map.remove(&key);
                            drop(map);
                            warn!(
                                stream_id = key.stream_id,
                                language = key.language,
                                "lsp server exited unexpectedly"
                            );
                            let _ = events.send(LspSessionEvent::SessionStatus {
                                stream_id: key.stream_id.clone(),
                                language: key.language.clone(),
                                status: LspSessionStatus::Crashed,
                                message: Some("language server exited".into()),
                            });
                        }
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "lsp session pump lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Issue a JSON-RPC request on the (lazily spawned) session.
    pub async fn request_session(
        &self,
        stream_id: &str,
        language: &str,
        cwd: PathBuf,
        method: &str,
        params: Value,
    ) -> Result<Value, LspSessionError> {
        let proxy = self.ensure(stream_id, language, cwd).await?;
        Ok(proxy.request(method, params).await?)
    }

    /// Send a JSON-RPC notification on the (lazily spawned) session.
    /// `textDocument/didOpen|didChange|didClose` are intercepted to
    /// keep the document mirror current before forwarding.
    pub async fn notify_session(
        &self,
        stream_id: &str,
        language: &str,
        cwd: PathBuf,
        method: &str,
        params: Value,
    ) -> Result<(), LspSessionError> {
        let key = SessionKey {
            stream_id: stream_id.to_string(),
            language: language.to_string(),
        };
        self.update_mirror(&key, method, &params);
        let proxy = self.ensure(stream_id, language, cwd).await?;
        Ok(proxy.notify(method, params).await?)
    }

    fn update_mirror(&self, key: &SessionKey, method: &str, params: &Value) {
        let uri = params
            .pointer("/textDocument/uri")
            .and_then(|u| u.as_str())
            .map(str::to_string);
        let Some(uri) = uri else { return };
        let mut docs = self.docs.lock().expect("docs mutex");
        match method {
            "textDocument/didOpen" => {
                let doc = MirrorDoc {
                    language_id: params
                        .pointer("/textDocument/languageId")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&key.language)
                        .to_string(),
                    version: params
                        .pointer("/textDocument/version")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1),
                    text: params
                        .pointer("/textDocument/text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                };
                docs.entry(key.clone()).or_default().insert(uri, doc);
            }
            "textDocument/didChange" => {
                // Full-text sync: the last content change wins.
                let text = params
                    .pointer("/contentChanges")
                    .and_then(|c| c.as_array())
                    .and_then(|a| a.last())
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str());
                let version = params
                    .pointer("/textDocument/version")
                    .and_then(|v| v.as_i64());
                if let Some(doc) = docs.entry(key.clone()).or_default().get_mut(&uri) {
                    if let Some(text) = text {
                        doc.text = text.to_string();
                    }
                    if let Some(v) = version {
                        doc.version = v;
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(m) = docs.get_mut(key) {
                    m.remove(&uri);
                }
            }
            _ => {}
        }
    }

    /// Tear down and respawn the session, replaying mirrored documents.
    pub async fn restart(
        &self,
        stream_id: &str,
        language: &str,
        cwd: PathBuf,
    ) -> Result<(), LspSessionError> {
        let key = SessionKey {
            stream_id: stream_id.to_string(),
            language: language.to_string(),
        };
        let existing = {
            let mut map = self.sessions.lock().await;
            map.remove(&key)
        };
        if let Some(entry) = existing {
            // Best-effort spec-compliant teardown; cap the wait so a
            // wedged server can't block the restart.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                entry.proxy.request("shutdown", Value::Null),
            )
            .await;
            let _ = entry.proxy.notify("exit", Value::Null).await;
        }
        self.ensure(stream_id, language, cwd).await?;
        let _ = self.events.send(LspSessionEvent::SessionStatus {
            stream_id: stream_id.to_string(),
            language: language.to_string(),
            status: LspSessionStatus::Restarted,
            message: None,
        });
        Ok(())
    }

    /// All known servers (yaml + installed, yaml wins per language),
    /// annotated with binary presence and live-session info.
    pub async fn list_servers(&self) -> Vec<LspServerListing> {
        let yaml: Vec<LspServerConfig> = self
            .config
            .read()
            .map(|c| c.lsp_servers.clone())
            .unwrap_or_default();
        let installed = self.installed.list();

        let mut out: Vec<LspServerListing> = Vec::new();
        for cfg in &yaml {
            out.push(LspServerListing {
                language_id: cfg.language_id.clone(),
                extensions: cfg.extensions.clone(),
                command: cfg.command.clone(),
                args: cfg.args.clone(),
                source: LspServerSource::Yaml,
                package_name: None,
                version: None,
                binary_exists: binary_exists(&cfg.command),
                running_streams: vec![],
                completion_trigger_characters: None,
            });
        }
        for entry in &installed {
            if yaml
                .iter()
                .any(|y| y.language_id == entry.config.language_id)
            {
                continue; // yaml wins
            }
            out.push(LspServerListing {
                language_id: entry.config.language_id.clone(),
                extensions: entry.config.extensions.clone(),
                command: entry.config.command.clone(),
                args: entry.config.args.clone(),
                source: LspServerSource::Installed,
                package_name: Some(entry.package.clone()),
                version: Some(entry.version.clone()),
                binary_exists: binary_exists(&entry.config.command),
                running_streams: vec![],
                completion_trigger_characters: None,
            });
        }

        let map = self.sessions.lock().await;
        for listing in &mut out {
            for (key, entry) in map.iter() {
                if key.language != listing.language_id {
                    continue;
                }
                listing.running_streams.push(key.stream_id.clone());
                if listing.completion_trigger_characters.is_none() {
                    listing.completion_trigger_characters = entry
                        .server_capabilities
                        .pointer("/completionProvider/triggerCharacters")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        });
                }
            }
            listing.running_streams.sort();
        }
        out.sort_by(|a, b| a.language_id.cmp(&b.language_id));
        out
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
                let _ = self.events.send(LspSessionEvent::SessionStatus {
                    stream_id: key.stream_id.clone(),
                    language: key.language.clone(),
                    status: LspSessionStatus::Stopped,
                    message: None,
                });
            }
        }
        drop(map);
        self.docs
            .lock()
            .expect("docs mutex")
            .retain(|k, _| k.stream_id != stream_id);
    }
}

/// Client capabilities advertised during `initialize`. Must stay in
/// step with what the Monaco providers + diagnostics pipeline actually
/// implement (see `.context/lsp.md`).
fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": {
                "didSave": true,
                "willSave": false,
                "willSaveWaitUntil": false,
            },
            "publishDiagnostics": { "relatedInformation": true },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": {},
            "references": {},
            "completion": {
                "completionItem": {
                    // Plain-text inserts only for now; flipping this on
                    // requires LSP-snippet → Monaco-snippet handling in
                    // the completion provider.
                    "snippetSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                },
                "contextSupport": true,
            },
            "rename": { "prepareSupport": false },
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract",
                            "refactor.inline", "refactor.rewrite", "source",
                            "source.organizeImports",
                        ]
                    }
                }
            },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": { "documentChanges": true },
            "configuration": true,
            "workspaceFolders": true,
        },
        "window": { "workDoneProgress": true },
    })
}

/// Answers for server→client requests we don't (yet) route to a real
/// implementation. Returning *something* matters more than the value:
/// servers block their own pipelines waiting on these.
fn auto_answer(method: &str, params: &Value) -> Value {
    match method {
        // "No per-item configuration" — one null per requested item.
        "workspace/configuration" => {
            let n = params
                .pointer("/items")
                .and_then(|i| i.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Value::Array(vec![Value::Null; n])
        }
        // We don't apply server-initiated workspace edits yet; saying
        // so honestly lets the server surface its own failure path.
        "workspace/applyEdit" => json!({ "applied": false }),
        // registerCapability / workDoneProgress/create / everything
        // else: null is the spec-blessed "ok, noted" for these.
        _ => Value::Null,
    }
}

/// Does `command` resolve to an executable? Absolute/relative paths are
/// checked directly; bare names are searched on `PATH`.
fn binary_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(command).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;
    use std::time::Duration;
    use tokio::time::timeout;

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

    /// Fake LSP server: answers initialize (echoing the client
    /// capabilities back inside the result so tests can assert on
    /// them), echoes "ping" notifications as "pong", asks
    /// `workspace/configuration` when poked with "askConfig" and
    /// reports the answer back via an "answeredConfig" notification.
    fn fake_server_config(language: &str) -> LspServerConfig {
        let script = r#"
import sys, json

def read_message():
    headers = b""
    while b"\r\n\r\n" not in headers:
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        headers += ch
    length = 0
    for line in headers.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))

def write_message(payload):
    body = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

opened = []
while True:
    msg = read_message()
    if msg is None:
        break
    method = msg.get("method")
    if "id" in msg and method == "initialize":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": {
            "capabilities": {"completionProvider": {"triggerCharacters": ["."]}},
            "clientCapabilities": msg["params"]["capabilities"],
        }})
    elif "id" in msg and method == "shutdown":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif method == "exit":
        break
    elif method == "die":
        sys.exit(1)
    elif method == "ping":
        write_message({"jsonrpc": "2.0", "method": "pong", "params": msg.get("params")})
    elif method == "askConfig":
        write_message({"jsonrpc": "2.0", "id": 7, "method": "workspace/configuration",
                       "params": {"items": [{"section": "a"}, {"section": "b"}]}})
    elif method == "textDocument/didOpen":
        opened.append(msg["params"]["textDocument"])
    elif method == "listOpened":
        write_message({"jsonrpc": "2.0", "method": "openedDocs", "params": {"docs": opened}})
    elif "id" in msg and "method" not in msg:
        write_message({"jsonrpc": "2.0", "method": "answeredConfig", "params": {"result": msg.get("result")}})
"#;
        LspServerConfig {
            language_id: language.into(),
            extensions: vec![],
            command: "python3".into(),
            args: vec!["-c".into(), script.to_string()],
        }
    }

    fn manager_with_fake(language: &str) -> LspSessionManager {
        let cfg = empty_config();
        cfg.write()
            .unwrap()
            .lsp_servers
            .push(fake_server_config(language));
        LspSessionManager::new(cfg)
    }

    async fn next_notification(
        rx: &mut broadcast::Receiver<LspSessionEvent>,
        want_method: &str,
    ) -> Value {
        loop {
            let evt = timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timely")
                .expect("ok");
            if let LspSessionEvent::ServerNotification { method, params, .. } = evt {
                if method == want_method {
                    return params;
                }
            }
        }
    }

    #[tokio::test]
    async fn ensure_returns_no_config_when_language_unknown() {
        let mgr = LspSessionManager::new(empty_config());
        let err = mgr
            .ensure("s-1", "tsx", std::env::temp_dir())
            .await
            .err()
            .expect("should error");
        assert!(matches!(err, LspSessionError::NoConfig(ref lang) if lang == "tsx"));
    }

    #[test]
    fn no_config_error_is_self_describing() {
        let err = LspSessionError::NoConfig("rust".into());
        let msg = err.to_string();
        assert!(msg.contains("rust-analyzer"), "got: {msg}");
        assert!(msg.contains("lsp_install_server"), "got: {msg}");
        assert!(msg.contains("oxplow.yaml"), "got: {msg}");
    }

    #[test]
    fn installed_servers_override_take_effect() {
        let mgr = LspSessionManager::new(empty_config());
        mgr.installed_servers().register(
            LspServerConfig {
                language_id: "rust".into(),
                extensions: vec!["rs".into()],
                command: "/tmp/fake/rust-analyzer".into(),
                args: vec![],
            },
            "rust-analyzer",
            "v1",
        );
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
        mgr.installed_servers().register(
            LspServerConfig {
                language_id: "rust".into(),
                extensions: vec!["rs".into()],
                command: "installed-rust-analyzer".into(),
                args: vec![],
            },
            "rust-analyzer",
            "v1",
        );
        assert_eq!(
            mgr.find_server_config("rust").unwrap().command,
            "yaml-rust-analyzer"
        );
    }

    #[test]
    fn unregister_package_removes_all_its_languages() {
        let servers = InstalledServers::default();
        servers.register(
            LspServerConfig {
                language_id: "c".into(),
                extensions: vec![],
                command: "clangd".into(),
                args: vec![],
            },
            "clangd",
            "v1",
        );
        servers.register(
            LspServerConfig {
                language_id: "cpp".into(),
                extensions: vec![],
                command: "clangd".into(),
                args: vec![],
            },
            "clangd",
            "v1",
        );
        servers.unregister_package("clangd");
        assert!(servers.list().is_empty());
    }

    #[tokio::test]
    async fn initialize_sends_real_capabilities() {
        let mgr = manager_with_fake("fake");
        let proxy = mgr
            .ensure("s-1", "fake", std::env::temp_dir())
            .await
            .expect("ensure");
        // The fake server echoes our declared client capabilities into
        // its initialize result; a second initialize round-trips them.
        let init = proxy
            .request(
                "initialize",
                json!({"capabilities": client_capabilities(), "params": {}}),
            )
            .await
            .expect("request");
        let echoed = init.get("clientCapabilities").cloned().unwrap_or_default();
        assert!(
            echoed.pointer("/textDocument/completion").is_some(),
            "expected non-empty client capabilities, got {echoed}"
        );
        assert_eq!(
            echoed.pointer("/textDocument/completion/completionItem/snippetSupport"),
            Some(&json!(false)),
        );
    }

    #[tokio::test]
    async fn server_notifications_surface_as_session_events() {
        let mgr = manager_with_fake("fake");
        let mut rx = mgr.subscribe();
        mgr.ensure("s-1", "fake", std::env::temp_dir())
            .await
            .expect("ensure");
        mgr.notify_session("s-1", "fake", std::env::temp_dir(), "ping", json!({"x": 1}))
            .await
            .expect("notify");
        let params = next_notification(&mut rx, "pong").await;
        assert_eq!(params, json!({"x": 1}));
    }

    #[tokio::test]
    async fn workspace_configuration_is_auto_answered() {
        let mgr = manager_with_fake("fake");
        let mut rx = mgr.subscribe();
        mgr.notify_session("s-1", "fake", std::env::temp_dir(), "askConfig", json!({}))
            .await
            .expect("notify");
        // The fake server only sends `answeredConfig` after receiving
        // our response to its workspace/configuration request.
        let params = next_notification(&mut rx, "answeredConfig").await;
        assert_eq!(params, json!({"result": [null, null]}));
    }

    #[tokio::test]
    async fn did_open_and_change_update_mirror_and_replay_on_restart() {
        let mgr = manager_with_fake("fake");
        let mut rx = mgr.subscribe();
        let cwd = std::env::temp_dir();
        mgr.notify_session(
            "s-1",
            "fake",
            cwd.clone(),
            "textDocument/didOpen",
            json!({"textDocument": {"uri": "file:///a.rs", "languageId": "fake", "version": 1, "text": "v1"}}),
        )
        .await
        .unwrap();
        mgr.notify_session(
            "s-1",
            "fake",
            cwd.clone(),
            "textDocument/didChange",
            json!({"textDocument": {"uri": "file:///a.rs", "version": 4}, "contentChanges": [{"text": "v4"}]}),
        )
        .await
        .unwrap();

        mgr.restart("s-1", "fake", cwd.clone()).await.unwrap();

        // The respawned server should have been replayed a didOpen with
        // the latest mirrored text + version.
        mgr.notify_session("s-1", "fake", cwd.clone(), "listOpened", json!({}))
            .await
            .unwrap();
        let params = next_notification(&mut rx, "openedDocs").await;
        let docs = params["docs"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["uri"], "file:///a.rs");
        assert_eq!(docs[0]["text"], "v4");
        assert_eq!(docs[0]["version"], 4);
    }

    #[tokio::test]
    async fn crash_emits_crashed_and_respawn_replays_docs() {
        let mgr = manager_with_fake("fake");
        let mut rx = mgr.subscribe();
        let cwd = std::env::temp_dir();
        mgr.notify_session(
            "s-1",
            "fake",
            cwd.clone(),
            "textDocument/didOpen",
            json!({"textDocument": {"uri": "file:///b.rs", "languageId": "fake", "version": 2, "text": "body"}}),
        )
        .await
        .unwrap();
        // Make the server exit abruptly.
        mgr.notify_session("s-1", "fake", cwd.clone(), "die", json!({}))
            .await
            .unwrap();
        let crashed = loop {
            let evt = timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("timely")
                .expect("ok");
            if let LspSessionEvent::SessionStatus { status, .. } = evt {
                if status == LspSessionStatus::Crashed {
                    break true;
                }
            }
        };
        assert!(crashed);

        // Next use respawns and replays the mirrored doc.
        mgr.notify_session("s-1", "fake", cwd.clone(), "listOpened", json!({}))
            .await
            .unwrap();
        let params = next_notification(&mut rx, "openedDocs").await;
        let docs = params["docs"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["uri"], "file:///b.rs");
        assert_eq!(docs[0]["text"], "body");
    }

    #[tokio::test]
    async fn list_servers_merges_yaml_and_installed_with_metadata() {
        let cfg = empty_config();
        cfg.write().unwrap().lsp_servers.push(LspServerConfig {
            language_id: "typescript".into(),
            extensions: vec!["ts".into()],
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
        });
        let mgr = LspSessionManager::new(cfg);
        mgr.installed_servers().register(
            LspServerConfig {
                language_id: "rust".into(),
                extensions: vec![],
                command: "/nonexistent/rust-analyzer".into(),
                args: vec![],
            },
            "rust-analyzer",
            "2026-01-01",
        );
        // Installed entry shadowed by yaml must not duplicate.
        mgr.installed_servers().register(
            LspServerConfig {
                language_id: "typescript".into(),
                extensions: vec![],
                command: "/elsewhere/tsserver".into(),
                args: vec![],
            },
            "typescript-language-server",
            "v9",
        );

        let listings = mgr.list_servers().await;
        assert_eq!(listings.len(), 2);
        let rust = listings.iter().find(|l| l.language_id == "rust").unwrap();
        assert_eq!(rust.source, LspServerSource::Installed);
        assert_eq!(rust.package_name.as_deref(), Some("rust-analyzer"));
        assert_eq!(rust.version.as_deref(), Some("2026-01-01"));
        assert!(!rust.binary_exists);
        let ts = listings
            .iter()
            .find(|l| l.language_id == "typescript")
            .unwrap();
        assert_eq!(ts.source, LspServerSource::Yaml);
        assert_eq!(ts.package_name, None);
    }

    #[tokio::test]
    async fn list_servers_reports_running_streams_and_trigger_chars() {
        let mgr = manager_with_fake("fake");
        mgr.ensure("s-9", "fake", std::env::temp_dir())
            .await
            .expect("ensure");
        let listings = mgr.list_servers().await;
        let fake = listings.iter().find(|l| l.language_id == "fake").unwrap();
        assert_eq!(fake.running_streams, vec!["s-9".to_string()]);
        assert_eq!(
            fake.completion_trigger_characters,
            Some(vec![".".to_string()])
        );
    }

    #[test]
    fn binary_exists_checks_path_and_bare_names() {
        assert!(binary_exists("/bin/sh") || binary_exists("/usr/bin/sh"));
        assert!(binary_exists("sh"));
        assert!(!binary_exists("definitely-not-a-real-binary-xyz"));
        assert!(!binary_exists("/nonexistent/path/to/server"));
    }
}
