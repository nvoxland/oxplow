//! A client of Codex's app server (`.context/agent-model.md` "Codex: an
//! app-server client"): JSON-RPC over a websocket on the shared daemon's
//! control socket, the way Codex's own TUI and IDE extension reach it.
//! Each user opens its own connection — a session's thread, one listing —
//! and the daemon outlives every one of them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

/// The largest message the client reads. A resumed thread's answer
/// carries its latest turns in full, tool output and patches included:
/// tens of megabytes, past the websocket's default 16 MiB frame limit.
const MAX_MESSAGE: usize = 256 << 20;

/// How long `codex app-server daemon start` may take.
const DAEMON_START: Duration = Duration::from_secs(30);

/// The daemon's control socket under Codex's home: `$CODEX_HOME`, else
/// `<home>/.codex`.
pub fn control_socket(codex_home: &Path) -> PathBuf {
    codex_home
        .join("app-server-control")
        .join("app-server-control.sock")
}

/// Codex's home for the person whose home is `home`.
pub fn codex_home(home: &Path) -> PathBuf {
    codex_home_in(home, std::env::var_os("CODEX_HOME"))
}

/// [`codex_home`] given the value of `$CODEX_HOME`.
fn codex_home_in(home: &Path, env: Option<std::ffi::OsString>) -> PathBuf {
    env.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
}

/// What the server sends that isn't an answer to one of ours.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Notification {
        method: String,
        params: Value,
    },
    /// A request for the client to answer ([`AppServer::respond`]): an
    /// approval.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

impl ServerMessage {
    pub fn params(&self) -> &Value {
        match self {
            ServerMessage::Notification { params, .. } | ServerMessage::Request { params, .. } => {
                params
            }
        }
    }

    /// The thread it is about, when it names one.
    pub fn thread_id(&self) -> Option<&str> {
        self.params().get("threadId").and_then(Value::as_str)
    }
}

type Answers = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// One connection to the daemon. Cheap to clone; the connection lives
/// while a clone or its message receiver does.
#[derive(Clone)]
pub struct AppServer {
    out: mpsc::UnboundedSender<Message>,
    answers: Answers,
    next_id: Arc<AtomicU64>,
    closed: watch::Receiver<bool>,
}

impl AppServer {
    /// Connect to the daemon at `socket`, starting it with `program` (`codex
    /// app-server daemon start`) when nothing answers, and run the
    /// handshake. The receiver gets every notification and server request,
    /// in arrival order.
    pub async fn connect(
        socket: &Path,
        program: Option<&Path>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        let stream = match tokio::net::UnixStream::connect(socket).await {
            Ok(s) => s,
            Err(first) => {
                let Some(program) = program else {
                    return Err(format!(
                        "Codex's app server isn't running ({}: {first})",
                        socket.display()
                    ));
                };
                start_daemon(program).await?;
                tokio::net::UnixStream::connect(socket).await.map_err(|e| {
                    format!(
                        "Codex's app server didn't come up at {}: {e}",
                        socket.display()
                    )
                })?
            }
        };
        Self::over(stream).await
    }

    /// Run the client over an open stream to the daemon's socket.
    async fn over(
        stream: tokio::net::UnixStream,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (ws, _) =
            tokio_tungstenite::client_async_with_config("ws://localhost/", stream, Some(config))
                .await
                .map_err(|e| format!("Codex's app server refused the connection: {e}"))?;
        let (mut sink, mut source) = ws.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let (messages, received) = mpsc::unbounded_channel();
        let answers: Answers = Arc::default();
        let (closed_tx, closed) = watch::channel(false);
        tokio::spawn(async move {
            while let Some(m) = outgoing.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        let pending = answers.clone();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = source.next().await {
                let text = match frame {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };
                if let Ok(v) = serde_json::from_str::<Value>(text.as_str()) {
                    route(v, &pending, &messages);
                }
            }
            for (_, waiting) in lock(&pending).drain() {
                let _ = waiting.send(Err("Codex's app server closed the connection".into()));
            }
            let _ = closed_tx.send(true);
        });
        let server = Self {
            out,
            answers,
            next_id: Arc::new(AtomicU64::new(1)),
            closed,
        };
        server
            .request(
                "initialize",
                json!({
                    "clientInfo": {"name": "oxplow", "title": "oxplow", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true},
                }),
            )
            .await?;
        server.notify("initialized", json!({}))?;
        Ok((server, received))
    }

    /// Send `method` and wait for its answer: its `result`, or its
    /// `error` as text.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.answers).insert(id, tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        rx.await
            .map_err(|_| "Codex's app server closed the connection".to_string())?
            .map_err(|e| format!("`{method}`: {e}"))
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Answer the server's request `id`.
    pub fn respond(&self, id: &Value, result: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// Refuse the server's request `id`.
    pub fn refuse(&self, id: &Value, message: &str) -> Result<(), String> {
        self.send(
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}),
        )
    }

    fn send(&self, v: Value) -> Result<(), String> {
        self.out
            .send(Message::text(v.to_string()))
            .map_err(|_| "Codex's app server closed the connection".to_string())
    }

    /// Resolves once the connection has closed.
    pub async fn closed(&self) {
        let mut closed = self.closed.clone();
        let _ = closed.wait_for(|c| *c).await;
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An answer to its waiting request, anything else to `messages`.
fn route(v: Value, answers: &Answers, messages: &mpsc::UnboundedSender<ServerMessage>) {
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match (method, v.get("id")) {
        (Some(method), Some(id)) => {
            let _ = messages.send(ServerMessage::Request {
                id: id.clone(),
                method,
                params,
            });
        }
        (Some(method), None) => {
            let _ = messages.send(ServerMessage::Notification { method, params });
        }
        (None, Some(id)) => {
            let Some(waiting) = id.as_u64().and_then(|id| lock(answers).remove(&id)) else {
                return;
            };
            let answer = match v.get("error") {
                Some(e) => Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| e.to_string())),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = waiting.send(answer);
        }
        (None, None) => {}
    }
}

async fn start_daemon(program: &Path) -> Result<(), String> {
    let run = tokio::process::Command::new(program)
        .args(["app-server", "daemon", "start"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(DAEMON_START, run)
        .await
        .map_err(|_| "starting Codex's app server timed out".to_string())?
        .map_err(|e| format!("starting Codex's app server: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "starting Codex's app server failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// A scripted app server on a unix socket, for tests here and in other
/// crates (feature `fake-app-server`): it answers `initialize` and every
/// other request with what `answer` says, records what clients send, and
/// pushes notifications and requests to every connected client.
#[cfg(any(test, feature = "fake-app-server"))]
pub mod fake {
    use super::*;
    use tokio::sync::broadcast;

    type Answer = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

    pub struct FakeAppServer {
        pub socket: PathBuf,
        push: broadcast::Sender<Value>,
        received: Arc<Mutex<Vec<Value>>>,
        arrived: Arc<tokio::sync::Notify>,
        /// Set by [`Self::stop`]: the accept loop and every connection end.
        down: tokio::sync::watch::Sender<bool>,
    }

    impl FakeAppServer {
        /// Listen at `<dir>/app-server-control/app-server-control.sock`, so
        /// `dir` stands in for Codex's home.
        pub fn start(
            dir: &Path,
            answer: impl Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static,
        ) -> Self {
            let socket = control_socket(dir);
            std::fs::create_dir_all(socket.parent().expect("a parent")).expect("socket dir");
            let listener = tokio::net::UnixListener::bind(&socket).expect("bind the socket");
            let (push, _) = broadcast::channel(256);
            let received = Arc::new(Mutex::new(Vec::new()));
            let arrived = Arc::new(tokio::sync::Notify::new());
            let answer: Arc<Answer> = Arc::new(answer);
            let (p, r, a) = (push.clone(), received.clone(), arrived.clone());
            let (down, mut stopped) = tokio::sync::watch::channel(false);
            let serving = down.subscribe();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { break };
                            tokio::spawn(serve(
                                stream,
                                answer.clone(),
                                p.subscribe(),
                                r.clone(),
                                a.clone(),
                                serving.clone(),
                            ));
                        }
                        _ = stopped.changed() => break,
                    }
                }
            });
            Self {
                socket,
                push,
                received,
                arrived,
                down,
            }
        }

        /// The daemon goes away: it stops listening and drops every
        /// client's connection.
        pub fn stop(&self) {
            let _ = self.down.send(true);
        }

        /// Send `message` (a notification or a request) to every client.
        pub fn push(&self, message: Value) {
            let _ = self.push.send(message);
        }

        /// Everything clients have sent, in order.
        pub fn received(&self) -> Vec<Value> {
            lock(&self.received).clone()
        }

        /// The first message a client sent that `pick` accepts, waiting
        /// (up to 5 s) for it.
        pub async fn wait_for(&self, pick: impl Fn(&Value) -> bool) -> Value {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let arrived = self.arrived.notified();
                if let Some(v) = self.received().into_iter().find(|v| pick(v)) {
                    return v;
                }
                if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                    panic!("nothing matching arrived; got {:?}", self.received());
                }
            }
        }
    }

    async fn serve(
        stream: tokio::net::UnixStream,
        answer: Arc<Answer>,
        mut push: broadcast::Receiver<Value>,
        received: Arc<Mutex<Vec<Value>>>,
        arrived: Arc<tokio::sync::Notify>,
        mut down: tokio::sync::watch::Receiver<bool>,
    ) {
        let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let (mut sink, mut source) = ws.split();
        loop {
            tokio::select! {
                _ = down.changed() => break,
                frame = source.next() => {
                    let Some(Ok(Message::Text(text))) = frame else { break };
                    let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                    lock(&received).push(v.clone());
                    arrived.notify_waiters();
                    let (Some(id), Some(method)) = (v.get("id"), v.get("method").and_then(Value::as_str)) else {
                        continue;
                    };
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    let reply = match method {
                        "initialize" => json!({"id": id, "result": {"userAgent": "fake"}}),
                        _ => match answer(method, &params) {
                            Ok(result) => json!({"id": id, "result": result}),
                            Err(message) => json!({"id": id, "error": {"code": -32600, "message": message}}),
                        },
                    };
                    if sink.send(Message::text(reply.to_string())).await.is_err() {
                        break;
                    }
                }
                pushed = push.recv() => {
                    let Ok(v) = pushed else { break };
                    if sink.send(Message::text(v.to_string())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeAppServer;
    use super::*;

    /// A unix socket's path has a length limit the system temp dir's
    /// would pass.
    fn short_dir() -> tempfile::TempDir {
        tempfile::Builder::new().tempdir_in("/tmp").unwrap()
    }

    /// The handshake, a request's result and error, a notification and a
    /// server request in order, an answer back, and the close.
    #[tokio::test]
    async fn requests_notifications_and_answers_cross_the_socket() {
        let home = short_dir();
        let fake = FakeAppServer::start(home.path(), |method, params| match method {
            "thread/list" => Ok(json!({"data": [], "cwd": params["cwd"]})),
            _ => Err(format!("no {method}")),
        });
        let (server, mut messages) = AppServer::connect(&fake.socket, None).await.unwrap();
        let sent = fake.received();
        assert_eq!(sent[0]["method"], "initialize");
        assert_eq!(sent[0]["params"]["capabilities"]["experimentalApi"], true);
        assert_eq!(
            server
                .request("thread/list", json!({"cwd": "/w"}))
                .await
                .unwrap()["cwd"],
            "/w"
        );
        assert_eq!(
            server.request("nope", json!({})).await.unwrap_err(),
            "`nope`: no nope"
        );
        fake.push(json!({"method": "turn/started", "params": {"threadId": "t1"}}));
        fake.push(json!({"id": 7, "method": "item/fileChange/requestApproval", "params": {"threadId": "t1"}}));
        let first = messages.recv().await.unwrap();
        assert_eq!(first.thread_id(), Some("t1"));
        assert!(
            matches!(&first, ServerMessage::Notification { method, .. } if method == "turn/started")
        );
        let ServerMessage::Request { id, method, .. } = messages.recv().await.unwrap() else {
            panic!("not a request")
        };
        assert_eq!(method, "item/fileChange/requestApproval");
        server.respond(&id, json!({"decision": "decline"})).unwrap();
        let answer = fake.wait_for(|v| v.get("id") == Some(&json!(7))).await;
        assert_eq!(answer["result"]["decision"], "decline");
        assert!(fake.received().iter().any(|v| v["method"] == "initialized"));
    }

    /// A resumed thread's answer runs to tens of megabytes (full turns
    /// with their tools' output): one past the websocket's default 16 MiB
    /// frame limit still arrives.
    #[tokio::test]
    async fn a_large_answer_arrives() {
        let home = short_dir();
        let big = "x".repeat(24 << 20);
        let fake = FakeAppServer::start(home.path(), move |_, _| Ok(json!({ "big": big })));
        let (server, _messages) = AppServer::connect(&fake.socket, None).await.unwrap();
        let answer = server.request("thread/resume", json!({})).await.unwrap();
        assert_eq!(answer["big"].as_str().map(str::len), Some(24 << 20));
    }

    /// Nothing listening and nothing to start it with is a clear error.
    #[tokio::test]
    async fn no_daemon_and_no_program_is_an_error() {
        let home = short_dir();
        let err = AppServer::connect(&control_socket(home.path()), None)
            .await
            .err()
            .unwrap();
        assert!(err.contains("isn't running"), "{err}");
    }

    /// With nothing listening, the daemon is started with `program` and
    /// connected to once it is up.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_daemon_is_started() {
        use std::os::unix::fs::PermissionsExt;
        let home = short_dir();
        let bin = home.path().join("codex");
        let marker = home.path().join("started");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\n[ \"$*\" = 'app-server daemon start' ] && touch '{}'\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The "daemon" the script starts is the fake, already listening
        // elsewhere: the connect still fails, after the start ran.
        let err = AppServer::connect(&control_socket(home.path()), Some(&bin))
            .await
            .err()
            .unwrap();
        assert!(marker.exists(), "the daemon was started");
        assert!(err.contains("didn't come up"), "{err}");
    }

    /// A custom config namespace: `$CODEX_HOME` moves the daemon's socket
    /// (and so the threads oxplow lists and opens) off the default; unset
    /// or empty it is `~/.codex`.
    #[test]
    fn codex_home_follows_the_environment_when_set() {
        let home = Path::new("/home/p");
        assert_eq!(codex_home_in(home, None), Path::new("/home/p/.codex"));
        assert_eq!(
            codex_home_in(home, Some("".into())),
            Path::new("/home/p/.codex")
        );
        let custom = codex_home_in(home, Some("/data/codex-work".into()));
        assert_eq!(custom, Path::new("/data/codex-work"));
        assert_eq!(
            control_socket(&custom),
            Path::new("/data/codex-work/app-server-control/app-server-control.sock")
        );
    }
}
