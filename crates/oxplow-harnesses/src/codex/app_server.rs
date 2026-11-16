//! A client of Codex's app server (`.context/agent-model.md` "Codex: an
//! app-server client"): JSON-RPC over a websocket on the shared daemon's
//! control socket, the way Codex's own TUI and IDE extension reach it.
//! Each user opens its own connection — a session's thread, one listing —
//! and the daemon outlives every one of them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

/// The largest message the client reads. A resumed thread's answer
/// carries its latest turns in full, tool output and patches included:
/// tens of megabytes, past the websocket's default 16 MiB frame limit.
#[cfg(unix)]
const MAX_MESSAGE: usize = 256 << 20;

/// How long `codex app-server daemon start` may take.
#[cfg(unix)]
const DAEMON_START: Duration = Duration::from_secs(30);

/// What a Codex session says on a platform without unix sockets: the
/// daemon's control socket is one, and a client of it isn't written for
/// anything else yet (Windows would need a named-pipe transport). Said
/// where a session starts, never worked around.
const NOT_ON_WINDOWS: &str = "Codex sessions aren't available on Windows yet";

/// Why Codex can't be reached on a platform with (`unix`) or without unix
/// sockets; `None` where it can.
fn unavailable_on(unix: bool) -> Option<&'static str> {
    (!unix).then_some(NOT_ON_WINDOWS)
}

/// Why Codex can't be reached on this platform; `None` where it can.
pub fn unavailable() -> Option<&'static str> {
    unavailable_on(cfg!(unix))
}

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
    recording: Option<Arc<Recording>>,
}

/// The file a session's traffic is recorded to: `OXPLOW_CODEX_DEBUG`, the
/// way `OXPLOW_HOOK_DEBUG` records the hook payloads of the other
/// harnesses. Unset or empty, nothing is recorded.
pub const RECORD_ENV: &str = "OXPLOW_CODEX_DEBUG";

#[cfg(unix)]
fn recording_from_env() -> Option<PathBuf> {
    std::env::var_os(RECORD_ENV)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// The connection that records what the daemon broadcasts: a session's.
/// Every connection receives each broadcast (`thread/started`,
/// `thread/status/changed`, …), so only this one writes what it hears; the
/// others write only what they send and what they are answered.
pub const OBSERVER: &str = "observer";
/// A short-lived connection that lists threads.
pub const LISTING: &str = "listing";

/// One connection's part in a recording: the file, a number that tells its
/// lines from other connections', and what it is for.
struct Recording {
    path: PathBuf,
    conn: u64,
    purpose: &'static str,
}

/// Held across every write to a recording file, from every connection.
static WRITE: Mutex<()> = Mutex::new(());
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// Writes one line, whole, under [`WRITE`]. Best-effort: a write that
/// fails is dropped, never an error for the session.
fn append(path: &Path, line: &Value) {
    let _held = lock(&WRITE);
    write_line(path, line);
}

/// One `write_all` of the whole line: lines of connections writing at once
/// can't interleave.
fn write_line(path: &Path, line: &Value) {
    use std::io::Write;
    let mut text = line.to_string();
    text.push('\n');
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

impl Recording {
    fn new(path: &Path, purpose: &'static str) -> Self {
        Self {
            path: path.to_path_buf(),
            conn: NEXT_CONN.fetch_add(1, Ordering::Relaxed),
            purpose,
        }
    }

    /// A line tagged with this connection (`conn`, `purpose`).
    fn write(&self, mut line: Value) {
        line["conn"] = json!(self.conn);
        line["purpose"] = json!(self.purpose);
        append(&self.path, &line);
    }

    /// Whether this connection records what the daemon sends it unasked.
    fn hears_broadcasts(&self) -> bool {
        self.purpose == OBSERVER
    }

    /// The first line of a new recording (a file that is empty or
    /// missing): when it was made, with which Codex (not known at connect
    /// time, so `null`), and a note for whoever trims it into a fixture.
    /// Checked and written under the lock, so only one connection writes it.
    #[cfg(unix)]
    fn begin(&self) {
        let _held = lock(&WRITE);
        if std::fs::metadata(&self.path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
        {
            return;
        }
        let today: String = oxplow_domain::Timestamp::now()
            .to_string()
            .chars()
            .take(10)
            .collect();
        write_line(
            &self.path,
            &json!({"recorded": today, "codex": Value::Null, "note": ""}),
        );
    }
}

#[cfg(unix)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl AppServer {
    fn record(&self, line: Value) {
        if let Some(recording) = &self.recording {
            recording.write(line);
        }
    }

    /// Connect to the daemon at `socket`, starting it with `program` (`codex
    /// app-server daemon start`) when nothing answers, and run the
    /// handshake. The receiver gets every notification and server request,
    /// in arrival order.
    #[cfg(not(unix))]
    pub async fn connect(
        _socket: &Path,
        _program: Option<&Path>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        Err(NOT_ON_WINDOWS.into())
    }

    /// See the unix `connect` above.
    #[cfg(unix)]
    pub async fn connect(
        socket: &Path,
        program: Option<&Path>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        Self::connect_for(socket, program, OBSERVER).await
    }

    /// See the unix `connect_for` below.
    #[cfg(not(unix))]
    pub async fn connect_for(
        _socket: &Path,
        _program: Option<&Path>,
        _purpose: &'static str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        Err(NOT_ON_WINDOWS.into())
    }

    /// [`Self::connect`] for a `purpose` ([`OBSERVER`], [`LISTING`]): what
    /// its lines in a recording are tagged with.
    #[cfg(unix)]
    pub async fn connect_for(
        socket: &Path,
        program: Option<&Path>,
        purpose: &'static str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ServerMessage>), String> {
        Self::connect_recording(socket, program, recording_from_env().as_deref(), purpose).await
    }

    /// [`Self::connect`], recording every message sent and received to
    /// `record` (`.context/agent-model.md` "Recording a Codex session").
    #[cfg(unix)]
    pub async fn connect_recording(
        socket: &Path,
        program: Option<&Path>,
        record: Option<&Path>,
        purpose: &'static str,
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
        Self::over(stream, record, purpose).await
    }

    /// Run the client over an open stream to the daemon's socket.
    #[cfg(unix)]
    async fn over(
        stream: tokio::net::UnixStream,
        record: Option<&Path>,
        purpose: &'static str,
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
        let recording = record.map(|path| Arc::new(Recording::new(path, purpose)));
        if let Some(recording) = &recording {
            recording.begin();
        }
        tokio::spawn(async move {
            while let Some(m) = outgoing.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        let pending = answers.clone();
        let heard = recording.clone();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = source.next().await {
                let text = match frame {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };
                if let Ok(v) = serde_json::from_str::<Value>(text.as_str()) {
                    if let Some(recording) = &heard {
                        // A broadcast arrives on every connection: one
                        // records it. Answers are each connection's own.
                        let answer = v.get("method").is_none();
                        if answer || recording.hears_broadcasts() {
                            recording.write(heard_line(&v));
                        }
                    }
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
            recording,
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
        if self.recording.is_some() {
            self.record(sent_line(&v));
        }
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

/// A message oxplow sends, in the fixtures' shape: a request or
/// notification is `{"sent": method, params}`; an answer to the server's
/// request is `{"answered": id, result | error}`.
fn sent_line(v: &Value) -> Value {
    match v.get("method").and_then(Value::as_str) {
        Some(method) => {
            json!({"sent": method, "params": v.get("params").cloned().unwrap_or(Value::Null)})
        }
        None => {
            let mut line = json!({"answered": v.get("id").cloned().unwrap_or(Value::Null)});
            for key in ["result", "error"] {
                if let Some(x) = v.get(key) {
                    line[key] = x.clone();
                }
            }
            line
        }
    }
}

/// A message the server sends, in the fixtures' shape: a notification is
/// `{method, params, emittedAtMs}` (a server request adds its `id`); an
/// answer to a request of oxplow's is `{"response": result}`, or
/// `{"error": message}`.
#[cfg(unix)]
fn heard_line(v: &Value) -> Value {
    if let Some(method) = v.get("method").and_then(Value::as_str) {
        let mut line = json!({
            "method": method,
            "params": v.get("params").cloned().unwrap_or(Value::Null),
            "emittedAtMs": now_ms(),
        });
        if let Some(id) = v.get("id") {
            line["id"] = id.clone();
        }
        return line;
    }
    match v.get("error") {
        Some(e) => json!({"error": e}),
        None => json!({"response": v.get("result").cloned().unwrap_or(Value::Null)}),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An answer to its waiting request, anything else to `messages`.
#[cfg(unix)]
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

#[cfg(unix)]
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
#[cfg(all(unix, any(test, feature = "fake-app-server")))]
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
mod platform_tests {
    use super::*;

    #[test]
    fn windows_says_plainly_that_codex_sessions_are_not_available() {
        let said = unavailable_on(false).expect("a message on a platform without unix sockets");
        assert!(said.contains("aren't available on Windows yet"), "{said}");
        assert_eq!(unavailable_on(true), None);
    }
}

#[cfg(test)]
#[cfg(unix)]
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

    /// With a recording file, every message either side sends lands in it in
    /// the fixtures' shape, after a header line; without one, nothing is
    /// written. A second connection to the same file adds no second header.
    #[tokio::test]
    async fn a_connection_records_its_traffic_in_the_fixtures_shape() {
        let home = short_dir();
        let fake = FakeAppServer::start(home.path(), |method, _| match method {
            "thread/list" => Ok(json!({"data": []})),
            _ => Err(format!("no {method}")),
        });
        let file = home.path().join("session.jsonl");
        let (server, mut messages) =
            AppServer::connect_recording(&fake.socket, None, Some(&file), OBSERVER)
                .await
                .unwrap();
        server
            .request("thread/list", json!({"cwd": "/w"}))
            .await
            .unwrap();
        server.request("nope", json!({})).await.unwrap_err();
        fake.push(json!({"method": "turn/started", "params": {"threadId": "t1"}}));
        fake.push(json!({"id": 7, "method": "item/fileChange/requestApproval", "params": {"threadId": "t1"}}));
        messages.recv().await.unwrap();
        let ServerMessage::Request { id, .. } = messages.recv().await.unwrap() else {
            panic!("not a request")
        };
        server.respond(&id, json!({"decision": "decline"})).unwrap();
        fake.wait_for(|v| v.get("id") == Some(&json!(7))).await;
        let (again, _) = AppServer::connect_recording(&fake.socket, None, Some(&file), LISTING)
            .await
            .unwrap();
        drop(again);

        let lines: Vec<Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(lines[0]["recorded"].is_string() && lines[0].get("note").is_some());
        assert_eq!(
            lines.iter().filter(|l| l.get("recorded").is_some()).count(),
            1,
            "one header"
        );
        let has = |pick: &dyn Fn(&Value) -> bool| lines.iter().any(pick);
        assert!(has(&|l| l["sent"] == "initialize"));
        assert!(has(
            &|l| l["sent"] == "thread/list" && l["params"]["cwd"] == "/w"
        ));
        assert!(has(&|l| l["response"] == json!({"data": []})));
        assert!(has(&|l| l.get("error").is_some()));
        assert!(has(&|l| l["method"] == "turn/started"
            && l["params"]["threadId"] == "t1"
            && l["emittedAtMs"].is_u64()));
        assert!(has(
            &|l| l["method"] == "item/fileChange/requestApproval" && l["id"] == 7
        ));
        assert!(has(
            &|l| l["answered"] == 7 && l["result"]["decision"] == "decline"
        ));

        let written = std::fs::read_to_string(&file).unwrap();
        let (quiet, _) = AppServer::connect_recording(&fake.socket, None, None, LISTING)
            .await
            .unwrap();
        quiet.request("thread/list", json!({})).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            written,
            "no recording, no writes"
        );
    }

    /// Connections recording to one file at once write only whole lines,
    /// each tagged with its connection, and a broadcast (which every
    /// connection receives) is in the file once, from the observer.
    #[tokio::test]
    async fn concurrent_connections_write_whole_tagged_lines_and_a_broadcast_once() {
        let home = short_dir();
        let fake = FakeAppServer::start(home.path(), |_, _| Ok(json!({"ok": true})));
        let file = home.path().join("session.jsonl");
        let (observer, mut heard) =
            AppServer::connect_recording(&fake.socket, None, Some(&file), OBSERVER)
                .await
                .unwrap();
        let mut listings = Vec::new();
        for _ in 0..3 {
            listings.push(
                AppServer::connect_recording(&fake.socket, None, Some(&file), LISTING)
                    .await
                    .unwrap(),
            );
        }
        let big = "x".repeat(20_000);
        for n in 0..40 {
            fake.push(json!({"method": "thread/status/changed", "params": {"threadId": "t", "n": n, "pad": big}}));
        }
        let mut asks = Vec::new();
        for (server, _) in &listings {
            let server = server.clone();
            let big = big.clone();
            asks.push(tokio::spawn(async move {
                for _ in 0..40 {
                    server
                        .request("thread/list", json!({"pad": big}))
                        .await
                        .unwrap();
                }
            }));
        }
        for _ in 0..40 {
            observer
                .request("thread/read", json!({"pad": big}))
                .await
                .unwrap();
        }
        for ask in asks {
            ask.await.unwrap();
        }
        for _ in 0..40 {
            heard.recv().await.unwrap();
        }
        for (_, rx) in &mut listings {
            for _ in 0..40 {
                rx.recv().await.unwrap();
            }
        }
        let lines: Vec<Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).expect("a whole JSON line"))
            .collect();
        let broadcasts: Vec<&Value> = lines
            .iter()
            .filter(|l| l["method"] == "thread/status/changed")
            .collect();
        assert_eq!(broadcasts.len(), 40, "once, not once per connection");
        assert!(broadcasts.iter().all(|l| l["purpose"] == "observer"));
        let conns: std::collections::HashSet<u64> = lines
            .iter()
            .filter(|l| l.get("recorded").is_none())
            .map(|l| l["conn"].as_u64().expect("tagged"))
            .collect();
        assert_eq!(conns.len(), 4, "every connection's own lines are there");
        assert!(lines
            .iter()
            .filter(|l| l["purpose"] == "listing")
            .all(|l| l.get("method").is_none()));
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
