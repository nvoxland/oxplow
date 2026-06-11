use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use crate::codec::{self, CodecError};

#[derive(Debug, Error)]
pub enum LspError {
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("server returned error {code}: {message}")]
    Server { code: i64, message: String },
    #[error("server closed before responding")]
    ServerClosed,
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dropped")]
    Dropped,
}

/// Spawn parameters for a language server.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Server-originated message: either a notification or a request the
/// server is making *of* the client (e.g.
/// `workspace/configuration`). The proxy itself doesn't answer these
/// — the consumer (oxplow-app) does.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Stream closed; the proxy is no longer usable.
    Closed,
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

/// Active LSP proxy. Drop to terminate.
pub struct LspProxy {
    next_id: AtomicI64,
    write_tx: mpsc::Sender<String>,
    pending: PendingMap,
    events: broadcast::Sender<ServerEvent>,
    _child: Child,
}

impl LspProxy {
    pub fn spawn(cfg: SpawnConfig) -> Result<Self, LspError> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = cfg.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn().map_err(|e| LspError::Spawn(e.to_string()))?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (write_tx, write_rx) = mpsc::channel::<String>(64);
        let (event_tx, _) = broadcast::channel::<ServerEvent>(256);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        // Writer task.
        tokio::spawn(writer_task(stdin, write_rx));

        // Reader task.
        tokio::spawn(reader_task(stdout, pending.clone(), event_tx.clone()));

        // Stderr drain so the child doesn't block on a full pipe.
        tokio::spawn(stderr_drain(stderr));

        Ok(Self {
            next_id: AtomicI64::new(1),
            write_tx,
            pending,
            events: event_tx,
            _child: child,
        })
    }

    /// Send a JSON-RPC request and await the response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        self.write_tx
            .send(frame)
            .await
            .map_err(|_| LspError::Dropped)?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(LspError::ServerClosed),
        }
    }

    /// Send a JSON-RPC notification (no reply expected).
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let frame = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string();
        self.write_tx
            .send(frame)
            .await
            .map_err(|_| LspError::Dropped)?;
        Ok(())
    }

    /// Answer a server-originated request (`ServerEvent::Request`).
    /// `id` must be the id the server sent; `result` is the response
    /// payload (use `Value::Null` when the method has no meaningful
    /// answer).
    pub async fn respond(&self, id: Value, result: Value) -> Result<(), LspError> {
        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        })
        .to_string();
        self.write_tx
            .send(frame)
            .await
            .map_err(|_| LspError::Dropped)?;
        Ok(())
    }

    /// Subscribe to server-originated events. Multiple subscribers
    /// allowed; events emitted before subscription are dropped.
    pub fn events(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }
}

async fn writer_task(mut stdin: tokio::process::ChildStdin, mut rx: mpsc::Receiver<String>) {
    while let Some(frame) = rx.recv().await {
        if let Err(e) = codec::write_framed(&mut stdin, &frame).await {
            warn!("lsp writer task error: {e}");
            break;
        }
    }
    let _ = stdin.shutdown().await;
}

async fn reader_task(
    mut stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    events: broadcast::Sender<ServerEvent>,
) {
    let mut buf = bytes::BytesMut::with_capacity(8192);
    let mut chunk = vec![0u8; 8192];
    let mut codec = crate::codec::LspCodec;

    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                warn!("lsp reader io error: {e}");
                break;
            }
        };
        buf.extend_from_slice(&chunk[..read]);

        loop {
            use tokio_util::codec::Decoder;
            let msg = match codec.decode(&mut buf) {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => {
                    warn!("lsp codec error: {e}");
                    break;
                }
            };
            handle_message(msg, &pending, &events).await;
        }
    }

    debug!("lsp reader exiting");
    let _ = events.send(ServerEvent::Closed);
}

async fn handle_message(
    raw: String,
    pending: &PendingMap,
    events: &broadcast::Sender<ServerEvent>,
) {
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            warn!("lsp received non-json: {e}");
            return;
        }
    };

    if let Some(id_val) = v.get("id") {
        // Either a server response (has result/error and id is a number
        // we issued) or a server-originated request (has method).
        if v.get("method").is_some() {
            let method = v["method"].as_str().unwrap_or("").to_string();
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            let _ = events.send(ServerEvent::Request {
                id: id_val.clone(),
                method,
                params,
            });
            return;
        }

        let Some(id_num) = id_val.as_i64() else {
            warn!("lsp response had non-integer id: {id_val}");
            return;
        };
        let mut map = pending.lock().await;
        let Some(tx) = map.remove(&id_num) else {
            warn!("lsp response for unknown id {id_num}");
            return;
        };
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("(no message)")
                .to_string();
            let _ = tx.send(Err(LspError::Server { code, message }));
        } else {
            let result = v.get("result").cloned().unwrap_or(Value::Null);
            let _ = tx.send(Ok(result));
        }
        return;
    }

    if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let _ = events.send(ServerEvent::Notification {
            method: method.to_string(),
            params,
        });
    }
}

async fn stderr_drain(mut stderr: tokio::process::ChildStderr) {
    let mut buf = vec![0u8; 4096];
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) => break,
            Ok(_) => {} // discard
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    /// Spawn a fake LSP server using a one-off shell script. The script
    /// reads framed JSON-RPC, identifies the `initialize` method, and
    /// sends back a minimal initialize result.
    fn fake_server_cfg() -> SpawnConfig {
        // Use python3 — present on every test runner we care about and
        // gives us a small portable script.
        let script = r#"
import sys
import json

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
    body = sys.stdin.buffer.read(length)
    return json.loads(body.decode("utf-8"))

def write_message(payload):
    body = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

while True:
    msg = read_message()
    if msg is None:
        break
    if "id" in msg and msg.get("method") == "initialize":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": {"capabilities": {}}})
    elif "id" in msg and msg.get("method") == "shutdown":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif msg.get("method") == "exit":
        break
    elif msg.get("method") == "ping":
        write_message({"jsonrpc": "2.0", "method": "pong", "params": msg.get("params")})
    elif msg.get("method") == "ask":
        # Server -> client request; once the client answers, echo the
        # answer back as a notification so the test can observe it.
        write_message({"jsonrpc": "2.0", "id": 999, "method": "client/ask", "params": {}})
    elif "id" in msg and "method" not in msg:
        write_message({"jsonrpc": "2.0", "method": "answered", "params": msg.get("result")})
"#;
        SpawnConfig {
            command: "python3".into(),
            args: vec!["-c".into(), script.to_string()],
            cwd: None,
        }
    }

    #[tokio::test]
    async fn initialize_round_trip() {
        let proxy = LspProxy::spawn(fake_server_cfg()).expect("spawn");
        let result = timeout(
            Duration::from_secs(5),
            proxy.request("initialize", json!({})),
        )
        .await
        .expect("timely")
        .expect("ok");
        assert!(result.get("capabilities").is_some());
    }

    #[tokio::test]
    async fn notification_emitted_to_event_stream() {
        let proxy = LspProxy::spawn(fake_server_cfg()).expect("spawn");
        let mut events = proxy.events();
        // Initialize first so the proxy is in a known state.
        proxy.request("initialize", json!({})).await.unwrap();
        // Send a "ping" notification — server replies with a "pong"
        // notification we should receive on the event channel.
        proxy.notify("ping", json!({"hi": 1})).await.unwrap();
        let evt = timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timely")
            .expect("ok");
        match evt {
            ServerEvent::Notification { method, params } => {
                assert_eq!(method, "pong");
                assert_eq!(params, json!({"hi": 1}));
            }
            other => panic!("expected Notification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn respond_answers_server_request() {
        let proxy = LspProxy::spawn(fake_server_cfg()).expect("spawn");
        let mut events = proxy.events();
        proxy.request("initialize", json!({})).await.unwrap();
        // Ask the server to issue a request *of* the client.
        proxy.notify("ask", json!({})).await.unwrap();
        let req = timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timely")
            .expect("ok");
        let (id, method) = match req {
            ServerEvent::Request { id, method, .. } => (id, method),
            other => panic!("expected Request, got {other:?}"),
        };
        assert_eq!(method, "client/ask");
        proxy.respond(id, json!({"answer": 42})).await.unwrap();
        // The fake server echoes our response back as an `answered`
        // notification carrying the result payload.
        let evt = timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("timely")
            .expect("ok");
        match evt {
            ServerEvent::Notification { method, params } => {
                assert_eq!(method, "answered");
                assert_eq!(params, json!({"answer": 42}));
            }
            other => panic!("expected Notification, got {other:?}"),
        }
    }
}
