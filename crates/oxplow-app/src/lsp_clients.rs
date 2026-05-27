//! Raw LSP client registry for the renderer-facing JSON-RPC bridge.
//!
//! The renderer (`apps/desktop/src/lsp.ts`) issues its own JSON-RPC
//! IDs and runs its own correlation. So we don't go through
//! `oxplow_lsp::LspProxy` (which would steal the correlation): we
//! spawn a child language server, pipe framed JSON in and out, and
//! forward every server-originated frame back to the renderer
//! verbatim. The Tauri layer subscribes to `LspClientRegistry::events`
//! and re-emits each `(client_id, message)` pair via a Tauri event
//! channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use futures::SinkExt;
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{debug, warn};

use oxplow_config::{LspServerConfig, OxplowConfig};
use oxplow_lsp::codec::LspCodec;

#[derive(Debug, Error)]
pub enum LspClientError {
    #[error("no language server configured for `{0}`")]
    NoConfig(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("client not found: {0}")]
    NotFound(String),
    #[error("send dropped")]
    Dropped,
}

/// Server-originated LSP frame, tagged with the client that produced
/// it. Forwarded to the renderer over a Tauri event channel.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LspBridgeEvent {
    #[serde(rename = "clientId")]
    pub client_id: String,
    pub message: String,
}

struct ClientEntry {
    write_tx: mpsc::Sender<String>,
    /// Reader/writer/stderr drain handles. Aborted on close.
    tasks: Vec<JoinHandle<()>>,
}

/// Cheap-to-clone — every method takes `&self` and the registry is
/// behind an `Arc<Mutex>`.
#[derive(Clone)]
pub struct LspClientRegistry {
    config: Arc<std::sync::RwLock<OxplowConfig>>,
    inner: Arc<Mutex<HashMap<String, ClientEntry>>>,
    events_tx: broadcast::Sender<LspBridgeEvent>,
}

impl LspClientRegistry {
    pub fn new(config: Arc<std::sync::RwLock<OxplowConfig>>) -> Self {
        let (events_tx, _) = broadcast::channel(512);
        Self {
            config,
            inner: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
        }
    }

    /// Subscribe to the merged stream of server-originated frames from
    /// every open client. The Tauri bridge calls this once at startup
    /// and forwards each event via `app.emit("lsp:event", …)`.
    pub fn subscribe(&self) -> broadcast::Receiver<LspBridgeEvent> {
        self.events_tx.subscribe()
    }

    fn find_server_config(&self, language: &str) -> Option<LspServerConfig> {
        let cfg = self.config.read().ok()?;
        cfg.lsp_servers
            .iter()
            .find(|s| s.language_id == language)
            .cloned()
    }

    /// Spawn a fresh language-server child. Each call yields a new
    /// `client_id`; closing one does not affect any other.
    pub async fn open(&self, language: &str, cwd: PathBuf) -> Result<String, LspClientError> {
        let server = self
            .find_server_config(language)
            .ok_or_else(|| LspClientError::NoConfig(language.to_string()))?;

        let mut child = Command::new(&server.command)
            .args(&server.args)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| LspClientError::Spawn(e.to_string()))?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (write_tx, write_rx) = mpsc::channel::<String>(64);
        let client_id = format!("lsp-{}", uuid::Uuid::new_v4().simple());

        let writer_task = spawn_writer(stdin, write_rx);
        let reader_task = spawn_reader(client_id.clone(), stdout, self.events_tx.clone());
        let stderr_task = spawn_stderr_drain(stderr);
        // Reap the child on exit so wait()s don't accumulate. The
        // process is owned by `kill_on_drop` semantics; we just need a
        // task that observes its exit.
        let exit_task = tokio::spawn(async move {
            let _ = child.wait().await;
        });

        let entry = ClientEntry {
            write_tx,
            tasks: vec![writer_task, reader_task, stderr_task, exit_task],
        };
        self.inner.lock().await.insert(client_id.clone(), entry);
        Ok(client_id)
    }

    /// Forward a raw JSON-RPC frame (one full message body, no
    /// headers) from the renderer to the language server.
    pub async fn send(&self, client_id: &str, frame: String) -> Result<(), LspClientError> {
        let map = self.inner.lock().await;
        let entry = map
            .get(client_id)
            .ok_or_else(|| LspClientError::NotFound(client_id.to_string()))?;
        entry
            .write_tx
            .send(frame)
            .await
            .map_err(|_| LspClientError::Dropped)
    }

    pub async fn close(&self, client_id: &str) -> Result<(), LspClientError> {
        let mut map = self.inner.lock().await;
        let entry = map
            .remove(client_id)
            .ok_or_else(|| LspClientError::NotFound(client_id.to_string()))?;
        for handle in entry.tasks {
            handle.abort();
        }
        Ok(())
    }
}

fn spawn_writer(stdin: ChildStdin, mut rx: mpsc::Receiver<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut framed = FramedWrite::new(stdin, LspCodec);
        while let Some(frame) = rx.recv().await {
            if let Err(e) = framed.send(frame.as_str()).await {
                warn!(error = %e, "lsp writer send error");
                break;
            }
        }
    })
}

fn spawn_reader(
    client_id: String,
    stdout: ChildStdout,
    events: broadcast::Sender<LspBridgeEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut framed = FramedRead::new(stdout, LspCodec);
        while let Some(item) = framed.next().await {
            match item {
                Ok(message) => {
                    let _ = events.send(LspBridgeEvent {
                        client_id: client_id.clone(),
                        message,
                    });
                }
                Err(e) => {
                    warn!(client_id, error = %e, "lsp reader codec error");
                    break;
                }
            }
        }
        debug!(client_id, "lsp reader exiting");
    })
}

fn spawn_stderr_drain(mut stderr: ChildStderr) -> JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 4096];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;

    fn empty_config() -> Arc<std::sync::RwLock<OxplowConfig>> {
        Arc::new(std::sync::RwLock::new(OxplowConfig {
            agent: AgentKind::Claude,
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

    fn echo_config() -> Arc<std::sync::RwLock<OxplowConfig>> {
        let script = r#"
import sys
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
    body = sys.stdin.buffer.read(length).decode("utf-8")
    return body
def write_message(s):
    body = s.encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()
while True:
    msg = read_message()
    if msg is None:
        break
    write_message(msg)
"#;
        Arc::new(std::sync::RwLock::new(OxplowConfig {
            agent: AgentKind::Claude,
            project_name: "p".into(),
            lsp_servers: vec![LspServerConfig {
                language_id: "echo".into(),
                command: "python3".into(),
                args: vec!["-c".into(), script.to_string()],
                extensions: vec![],
            }],
            agent_prompt_append: String::new(),
            snapshot_retention_days: 7,
            generated: vec![],
            snapshot_max_file_bytes: 0,
            inject_session_context: true,
            collection: Default::default(),
        }))
    }

    #[tokio::test]
    async fn open_unknown_language_errors() {
        let reg = LspClientRegistry::new(empty_config());
        let err = reg.open("rust", std::env::temp_dir()).await.err().unwrap();
        assert!(matches!(err, LspClientError::NoConfig(l) if l == "rust"));
    }

    #[tokio::test]
    async fn round_trip_through_echo_server() {
        use std::time::Duration;
        let reg = LspClientRegistry::new(echo_config());
        let mut events = reg.subscribe();
        let id = reg.open("echo", std::env::temp_dir()).await.expect("open");
        reg.send(&id, r#"{"hello":1}"#.to_string())
            .await
            .expect("send");
        let evt = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timely")
            .expect("ok");
        assert_eq!(evt.client_id, id);
        assert_eq!(evt.message, r#"{"hello":1}"#);
        reg.close(&id).await.expect("close");
    }

    #[tokio::test]
    async fn close_unknown_client_errors() {
        let reg = LspClientRegistry::new(empty_config());
        let err = reg.close("nope").await.err().unwrap();
        assert!(matches!(err, LspClientError::NotFound(_)));
    }
}
