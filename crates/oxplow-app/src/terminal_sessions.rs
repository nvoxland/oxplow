//! Terminal session registry powering the renderer's terminal view (`terminal/useTerminalSession.ts`).
//!
//! Each session bridges an xterm.js instance in the renderer to a
//! process in a PTY on the host — a shell, or an agent CLI run directly
//! (no terminal multiplexer, tsk1018). The renderer
//! talks a small JSON protocol:
//!
//! - Outgoing (renderer → daemon):
//!   - `{type:"input", bytes:base64}` — user keystrokes
//!   - `{type:"input-binary", bytes:base64}` — binary input (paste)
//!   - `{type:"resize", cols, rows}` — viewport changed
//!
//! - Incoming (daemon → renderer):
//!   - `{type:"data", bytes:base64}` — bytes from the PTY
//!   - `{type:"exit", exitCode}` — the process ended
//!
//! Implementation: spawn the request via `oxplow_pty::PtyManager`; PTY
//! bytes flow back as `data` events. Scrollback is xterm.js's own, plus
//! the replay buffer a re-attach starts from.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
use oxplow_domain::{ThreadId, Timestamp};
pub use oxplow_pty::SpawnRequest;
use oxplow_pty::{PaneEvent, PaneId, PtyManager};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// Per-session replay buffer cap. Mirrors the renderer-era
/// `AgentPty.maxBytes` (~4 MiB) — enough for a generous scrollback
/// when the user comes back to a long-running thread, small enough
/// not to balloon memory across many idle threads.
const MAX_RING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum TerminalSessionError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("pty: {0}")]
    Pty(#[from] oxplow_pty::PtyError),
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("base64: {0}")]
    Base64(String),
}

/// Server-originated frame, tagged with the originating session.
/// Forwarded to the renderer over `pty:event`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TerminalBridgeEvent {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// JSON-encoded message body using the protocol above.
    pub message: String,
}

/// Identity of a long-lived terminal session. Two `attach` calls with
/// the same key resolve to the same PTY (and the same replay buffer)
/// so navigating between threads / streams doesn't kill the running
/// agent. Keys are opaque strings agreed-on by the IPC layer.
pub type SessionKey = String;

struct SessionEntry {
    pane_id: PaneId,
    /// OS pid of the spawned child (the shell, for the `shell` pane).
    /// Used to read the live cwd for terminal file-link resolution.
    pid: Option<u32>,
    forwarder: JoinHandle<()>,
    /// Replay buffer of recent PTY output. Bounded by `MAX_RING_BYTES`
    /// — the oldest chunks get evicted when the buffer would exceed
    /// the cap. Used to backfill a fresh `xterm.js` when the renderer
    /// re-attaches to a session that has been running in the
    /// background.
    ring: Arc<Mutex<RingBuffer>>,
    /// External key (stream/thread/pane/transport tuple) so we can
    /// drop the index entry when the session is explicitly killed.
    key: SessionKey,
}

struct RingBuffer {
    chunks: VecDeque<Bytes>,
    bytes: usize,
}

impl RingBuffer {
    fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
        }
    }

    fn push(&mut self, chunk: Bytes) {
        self.bytes += chunk.len();
        self.chunks.push_back(chunk);
        while self.bytes > MAX_RING_BYTES && self.chunks.len() > 1 {
            if let Some(old) = self.chunks.pop_front() {
                self.bytes -= old.len();
            }
        }
        if self.bytes > MAX_RING_BYTES && self.chunks.len() == 1 {
            // Single chunk over cap: trim from the left.
            if let Some(only) = self.chunks.pop_front() {
                let keep_from = only.len().saturating_sub(MAX_RING_BYTES);
                let trimmed = only.slice(keep_from..);
                self.bytes = trimmed.len();
                self.chunks.push_back(trimmed);
            }
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes);
        for chunk in &self.chunks {
            out.extend_from_slice(chunk);
        }
        out
    }
}

/// The registry key of agent session `session`'s PTY: one per session, so
/// a re-attach — another window, a browser client — resumes the one live
/// agent rather than spawning a duplicate in the same worktree.
pub fn agent_pane_key(session: oxplow_domain::AgentSessionId) -> SessionKey {
    format!("session|{session}")
}

/// The registry key of terminal `terminal`'s shell: its URI, so a
/// re-attach — another window, a browser client — resumes the one live
/// shell, and closing the terminal ends it.
pub fn terminal_key(terminal: oxplow_domain::TerminalId) -> SessionKey {
    oxplow_domain::uri::build::terminal_uri(terminal)
}

/// The agent a PTY runs: its thread and agent session. Its output stamps
/// the session's liveness, and its exit ends the session's harness session
/// (`HookIngestService` records a `SessionEnd` for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPane {
    pub thread: ThreadId,
    pub session: Option<oxplow_domain::AgentSessionId>,
    /// How its process was launched, to say what an exit meant.
    pub launch: crate::session_outcome::LaunchInfo,
}

#[derive(Clone)]
pub struct TerminalSessionRegistry {
    pty: PtyManager,
    inner: Arc<Mutex<HashMap<String, SessionEntry>>>,
    /// External-key → session_id index. Lets `attach_or_create`
    /// look up an existing session for a given (stream, thread,
    /// pane, transport) tuple in O(1).
    by_key: Arc<Mutex<HashMap<SessionKey, String>>>,
    events_tx: broadcast::Sender<TerminalBridgeEvent>,
    /// Per-thread PTY liveness. The forwarder stamps it on every output
    /// burst (for sessions spawned with a known thread id) so the stall
    /// watchdog can tell a busy long turn from a dead one — see
    /// [`crate::output_activity`] and tsk141.
    activity: crate::output_activity::OutputActivity,
    /// Where an agent pane's exit is recorded; set once the ingest is
    /// built (`ingest_exits_into`).
    exits: Arc<std::sync::OnceLock<crate::hook_ingest::HookIngestService>>,
    /// Each terminal's input queue, by key ([`Self::enqueue_input`]): its
    /// drain task is the only writer of input to the terminal's PTY.
    input: Arc<std::sync::Mutex<HashMap<SessionKey, mpsc::UnboundedSender<String>>>>,
    /// Signalled each time a session is registered, so a queue waiting
    /// for its terminal's PTY looks again.
    ready: Arc<tokio::sync::Notify>,
    /// The runtime the drain tasks run on: [`Self::enqueue_input`] is
    /// called from outside it (Tauri runs a sync command on its main
    /// thread).
    runtime: Option<tokio::runtime::Handle>,
    /// How long input waits for its terminal's PTY before it is dropped.
    input_wait: Duration,
}

/// How long input written for a terminal waits for its PTY (typed while it
/// spawns) before it is dropped: the launch failed.
const INPUT_WAIT: Duration = Duration::from_secs(10);

impl TerminalSessionRegistry {
    pub fn new(pty: PtyManager, activity: crate::output_activity::OutputActivity) -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            pty,
            inner: Arc::new(Mutex::new(HashMap::new())),
            by_key: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            activity,
            exits: Arc::new(std::sync::OnceLock::new()),
            input: Arc::new(std::sync::Mutex::new(HashMap::new())),
            ready: Arc::new(tokio::sync::Notify::new()),
            runtime: tokio::runtime::Handle::try_current().ok(),
            input_wait: INPUT_WAIT,
        }
    }

    /// The registry with input waiting `wait` for its PTY (tests).
    pub fn with_input_wait(mut self, wait: Duration) -> Self {
        self.input_wait = wait;
        self
    }

    /// Queue one input message (`input`, `input-binary` or `resize`, as
    /// [`Self::send`] reads it) for the terminal under `key`. Never blocks:
    /// the key's drain task writes its messages to the PTY in the order
    /// they were queued, holding them while the PTY is not there yet (up
    /// to the input wait, then dropping them). The person's keystrokes
    /// only: never an automation API (`.context/agent-model.md`).
    pub fn enqueue_input(&self, key: SessionKey, message: String) {
        let mut queues = self.input.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = queues.get(&key) {
            if tx.send(message.clone()).is_ok() {
                return;
            }
        }
        let Some(runtime) = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        else {
            warn!(%key, "terminal input dropped: no runtime to write it on");
            return;
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(message);
        queues.insert(key.clone(), tx);
        runtime.spawn(self.clone().drain_input(key, rx));
    }

    /// A terminal's input queue: each message, in order, to its live PTY.
    async fn drain_input(self, key: SessionKey, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(message) = rx.recv().await {
            let mut written = false;
            // A session that ends under the write is waited for once more
            // (Start Again replaces it).
            for _ in 0..2 {
                let Some(id) = self.wait_for_session(&key).await else {
                    break;
                };
                match self.send(&id, &message).await {
                    Err(TerminalSessionError::NotFound(_)) => continue,
                    Err(e) => warn!(%key, error = %e, "terminal input not written"),
                    Ok(()) => {}
                }
                written = true;
                break;
            }
            if !written {
                let mut dropped = 1;
                while rx.try_recv().is_ok() {
                    dropped += 1;
                }
                warn!(%key, dropped, "terminal input dropped: its PTY never came");
            }
        }
    }

    /// The live session under `key`, waiting up to the input wait for one
    /// to be registered.
    async fn wait_for_session(&self, key: &str) -> Option<String> {
        let deadline = tokio::time::Instant::now() + self.input_wait;
        loop {
            let notified = self.ready.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(id) = self.session_id_for_key(key).await {
                return Some(id);
            }
            tokio::time::timeout_at(deadline, notified).await.ok()?;
        }
    }

    /// Record each agent pane's process exit through `ingest` (a
    /// `SessionEnd` naming its agent session): a harness that posts no
    /// SessionEnd of its own still ends its session and its open turn.
    pub fn ingest_exits_into(&self, ingest: crate::hook_ingest::HookIngestService) {
        let _ = self.exits.set(ingest);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalBridgeEvent> {
        self.events_tx.subscribe()
    }

    /// Publish a bridge event without driving a real PTY. Used by the
    /// daemon's `/events` contract test to assert the `terminal` frame
    /// shape; mirrors `LspSessionManager::emit_event_for_tests`.
    pub fn emit_event_for_tests(&self, event: TerminalBridgeEvent) {
        let _ = self.events_tx.send(event);
    }

    /// Result of an attach: the session id (stable across reattaches)
    /// plus a base64-encoded snapshot of the replay buffer that the
    /// renderer should write into a fresh xterm before it starts
    /// consuming live events.
    pub fn build_attach_result(session_id: String, replay: Vec<u8>) -> AttachResult {
        AttachResult {
            session_id,
            replay_b64: B64.encode(&replay[..]),
        }
    }

    /// Look up an existing session by `key`; if none exists, build a
    /// `SpawnRequest` via `make_request` and spawn one. Returns the
    /// session id plus a snapshot of the live ring buffer (for replay
    /// when re-attaching to a long-running session). Mirrors the main
    /// branch's `AgentPtyStore.ensure` behavior.
    pub async fn attach_or_create(
        &self,
        key: SessionKey,
        cols: u16,
        rows: u16,
        make_request: impl FnOnce(u16, u16) -> SpawnRequest,
    ) -> Result<AttachResult, TerminalSessionError> {
        self.attach_or_create_for_agent(key, None, cols, rows, make_request)
            .await
    }

    /// As [`attach_or_create`], but binds the PTY to the agent it runs, so
    /// the forwarder records liveness for the stall watchdog and its exit
    /// ends the agent's session. Shell panes pass `None`.
    pub async fn attach_or_create_for_agent(
        &self,
        key: SessionKey,
        agent: Option<AgentPane>,
        cols: u16,
        rows: u16,
        make_request: impl FnOnce(u16, u16) -> SpawnRequest,
    ) -> Result<AttachResult, TerminalSessionError> {
        // Fast path: existing session for this key — take the attaching
        // pane's size (it may differ from the one the process last saw, e.g.
        // after an app restart) and replay its buffer.
        if let Some(existing_id) = self.by_key.lock().await.get(&key).cloned() {
            let map = self.inner.lock().await;
            if let Some(entry) = map.get(&existing_id) {
                self.pty.resize(&entry.pane_id, cols, rows).await?;
                let replay = entry.ring.lock().await.snapshot();
                return Ok(Self::build_attach_result(existing_id, replay));
            }
            // Stale by_key entry (entry was killed); fall through and
            // create a fresh session.
        }
        let req = make_request(cols, rows);
        let session_id = self.spawn_with(req, key.clone(), agent).await?;
        Ok(Self::build_attach_result(session_id, Vec::new()))
    }

    /// Read-only: the live session id indexed under external `key`, or
    /// `None` when none is registered (or the indexed session was
    /// already killed, leaving a stale `by_key` entry). **Never
    /// spawns** — it only reads the index, so a caller can resolve a
    /// thread's agent PTY without going
    /// through the spawn-capable `attach_or_create` path (tsk139).
    pub async fn session_id_for_key(&self, key: &str) -> Option<String> {
        let session_id = self.by_key.lock().await.get(key).cloned()?;
        // Validate the session is still live; a stale by_key entry
        // (its PTY was killed) must read as "no live session".
        if self.inner.lock().await.contains_key(&session_id) {
            Some(session_id)
        } else {
            None
        }
    }

    async fn spawn_with(
        &self,
        req: SpawnRequest,
        key: SessionKey,
        agent: Option<AgentPane>,
    ) -> Result<String, TerminalSessionError> {
        let mut handle = self.pty.spawn_pane(req).await?;
        let pane_id = handle.id.clone();
        let pid = handle.pid;
        let session_id = format!("term-{}", uuid::Uuid::new_v4().simple());
        let launched_at = Timestamp::now();
        if let (Some(agent_session), Some(pid), Some(ingest)) =
            (agent.and_then(|pane| pane.session), pid, self.exits.get())
        {
            if let Err(err) = ingest.record_agent_pid(agent_session, pid).await {
                warn!(?err, "recording an agent's pid failed");
            }
        }

        // Spawn a forwarder that pumps PaneEvents → TerminalBridgeEvents
        // and tees a copy into the session's replay buffer so any
        // future re-attach starts from the same screen state.
        let session_id_for_task = session_id.clone();
        let events = self.events_tx.clone();
        let ring = Arc::new(Mutex::new(RingBuffer::new()));
        let ring_for_task = Arc::clone(&ring);
        let activity = self.activity.clone();
        let exits = Arc::clone(&self.exits);
        let sessions = Arc::clone(&self.inner);
        let by_key = Arc::clone(&self.by_key);
        let input = Arc::clone(&self.input);
        let pty = self.pty.clone();
        let key_for_task = key.clone();
        // Held until the entry is inserted, so a process that exits at once
        // is unregistered only after it was registered.
        let mut map = self.inner.lock().await;
        let forwarder = tokio::spawn(async move {
            loop {
                match handle.events.recv().await {
                    Ok(PaneEvent::Output(bytes)) => {
                        // Stamp PTY liveness for the stall watchdog. Only
                        // thread-bound (agent) panes record; the cadence
                        // distinguishes a busy long turn from a dead one
                        // (tsk141).
                        if let Some(session) = agent.and_then(|pane| pane.session) {
                            activity.record(session, Timestamp::now());
                        }
                        ring_for_task.lock().await.push(bytes.clone());
                        let msg = serde_json::json!({
                            "type": "data",
                            "bytes": B64.encode(&bytes[..]),
                        })
                        .to_string();
                        let _ = events.send(TerminalBridgeEvent {
                            session_id: session_id_for_task.clone(),
                            message: msg,
                        });
                    }
                    Ok(PaneEvent::Exit { exit_code }) => {
                        let outcome = match (agent, exits.get()) {
                            (
                                Some(AgentPane {
                                    session: Some(s),
                                    launch,
                                    ..
                                }),
                                Some(ingest),
                            ) => ingest.exit_outcome(s, launch, launched_at, exit_code).await,
                            _ => crate::session_outcome::ExitOutcome::Ended,
                        };
                        let msg = serde_json::json!({
                            "type": "exit",
                            "exitCode": exit_code,
                            "outcome": outcome,
                        })
                        .to_string();
                        let _ = events.send(TerminalBridgeEvent {
                            session_id: session_id_for_task.clone(),
                            message: msg,
                        });
                        // Its process is gone: unregister it, so attaching
                        // to its key starts a fresh one (tsk1026).
                        if let Some(entry) = sessions.lock().await.remove(&session_id_for_task) {
                            let _ = pty.kill(&entry.pane_id).await;
                        }
                        let mut keys = by_key.lock().await;
                        if keys.get(&key_for_task) == Some(&session_id_for_task) {
                            keys.remove(&key_for_task);
                            // Keys typed after the exit wait for the next
                            // process in a fresh queue.
                            input
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&key_for_task);
                        }
                        drop(keys);
                        if let (Some(pane), Some(ingest)) = (agent, exits.get()) {
                            let exit = crate::hook_ingest::HookEnvelope {
                                session_title: None,
                                turn_run: None,
                                kind: oxplow_domain::HookKind::SessionEnd,
                                thread_id: Some(pane.thread),
                                stream_id: None,
                                agent_session_id: pane.session,
                                session_id: None,
                                payload_json: serde_json::json!({ "reason": "exit" }).to_string(),
                                prompt: None,
                                decision: None,
                                tool: None,
                                subagent: None,
                                peer: None,
                            };
                            if let Err(err) = ingest.ingest(exit).await {
                                warn!(?err, "recording an agent's exit failed");
                            }
                        }
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "terminal forwarder lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("terminal pane channel closed");
                        break;
                    }
                }
            }
        });

        map.insert(
            session_id.clone(),
            SessionEntry {
                pane_id,
                pid,
                forwarder,
                ring,
                key: key.clone(),
            },
        );
        self.by_key.lock().await.insert(key, session_id.clone());
        drop(map);
        self.ready.notify_waiters();
        Ok(session_id)
    }

    /// The OS pid of a live session's child process.
    pub async fn pid_of(&self, session_id: &str) -> Option<u32> {
        self.inner.lock().await.get(session_id)?.pid
    }

    /// Best-effort live working directory of a session's child process:
    /// for the `shell` pane the child IS the shell, so this tracks `cd`.
    /// Returns `None` on any failure (no pid, process gone, unsupported
    /// platform) — callers fall back to the worktree root.
    pub async fn session_cwd(&self, session_id: &str) -> Option<std::path::PathBuf> {
        let pid = { self.inner.lock().await.get(session_id)?.pid? };
        tokio::task::spawn_blocking(move || read_process_cwd(pid))
            .await
            .ok()
            .flatten()
    }

    /// Write one input message to a live session's PTY. Only the input
    /// queue's drain task calls it ([`Self::enqueue_input`]), and the
    /// paste-order tests.
    pub async fn send(&self, session_id: &str, message: &str) -> Result<(), TerminalSessionError> {
        let parsed: serde_json::Value = serde_json::from_str(message)
            .map_err(|e| TerminalSessionError::InvalidMessage(e.to_string()))?;
        let kind = parsed
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let pane_id = {
            let map = self.inner.lock().await;
            let entry = map
                .get(session_id)
                .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
            entry.pane_id.clone()
        };

        match kind.as_str() {
            "input" | "input-binary" => {
                let b64 = parsed
                    .get("bytes")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TerminalSessionError::InvalidMessage("missing bytes".into()))?;
                let raw = B64
                    .decode(b64)
                    .map_err(|e| TerminalSessionError::Base64(e.to_string()))?;
                self.pty.write(&pane_id, Bytes::from(raw)).await?;
            }
            "resize" => {
                let cols = parsed.get("cols").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                let rows = parsed.get("rows").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                // Reject absurdly-small resizes: a hidden xterm can fit
                // down to two cells, which would reflow the process's
                // output to that width.
                if cols < 20 || rows < 5 {
                    return Ok(());
                }
                self.pty.resize(&pane_id, cols, rows).await?;
            }
            other => {
                debug!(message_type = %other, "unhandled terminal message");
            }
        }
        Ok(())
    }

    /// Detach a renderer from a session without killing the
    /// underlying PTY. The session keeps running in the background;
    /// reattaching via `attach_or_create` resumes it with replay.
    /// Used when the renderer navigates away from a thread but the
    /// agent should keep working.
    pub async fn detach(&self, _session_id: &str) -> Result<(), TerminalSessionError> {
        // The forwarder is keyed off the broadcast channel, not a
        // particular renderer; nothing to do beyond accept the call.
        // (Kept as a method so the IPC surface and the renderer have
        // a clear "detach != close" contract.)
        Ok(())
    }

    /// Permanently kill a session and free its PTY. Use when a thread
    /// is closed or the user explicitly asks to terminate the agent —
    /// not on every renderer unmount.
    /// Stop the PTY registered under `key`, if one is.
    pub async fn close_key(&self, key: &str) {
        if let Some(id) = self.session_id_for_key(key).await {
            let _ = self.close(&id).await;
        }
    }

    /// Stop every session's PTY, as [`Self::close`] does one, and wait for
    /// their processes to be gone: the daemon's exit (`Services::shutdown`)
    /// runs no destructors, and each PTY leads a process group of its own
    /// that no signal to the daemon's group reaches.
    pub async fn close_all(&self) {
        let entries: Vec<SessionEntry> = self.inner.lock().await.drain().map(|(_, e)| e).collect();
        for entry in &entries {
            entry.forwarder.abort();
        }
        self.by_key.lock().await.clear();
        self.input.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let _ = self.pty.shutdown().await;
    }

    pub async fn close(&self, session_id: &str) -> Result<(), TerminalSessionError> {
        let mut map = self.inner.lock().await;
        let entry = map
            .remove(session_id)
            .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
        entry.forwarder.abort();
        let _ = self.pty.kill(&entry.pane_id).await;
        self.by_key.lock().await.remove(&entry.key);
        self.input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&entry.key);
        Ok(())
    }
}

/// Result of `attach_or_create` — the session id plus a base64
/// snapshot of the replay buffer that the renderer should write into
/// a fresh xterm before starting to consume live events.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AttachResult {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "replayB64")]
    pub replay_b64: String,
}

/// Read a process's current working directory by pid. Linux reads the
/// `/proc/<pid>/cwd` symlink; macOS shells out to `lsof` (no extra crate, and
/// `lsof` ships with the OS). Returns `None` on any failure. Runs blocking, so
/// callers invoke it via `spawn_blocking`.
fn read_process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(target_os = "macos")]
    {
        // `-Fn` is machine-readable: each field on its own line, prefixed by a
        // type char. The cwd fd (`-d cwd`) yields one `n<path>` line.
        let output = std::process::Command::new("lsof")
            .args(["-a", "-d", "cwd", "-p", &pid.to_string(), "-Fn"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.strip_prefix('n'))
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `{type:"input", bytes:<base64>}` message the way the
    /// renderer's `terminalInput` does for a paste.
    fn input_message(raw: &[u8]) -> String {
        let b64 = B64.encode(raw);
        serde_json::json!({ "type": "input", "bytes": b64 }).to_string()
    }

    /// Poll `path` until every `needle` is present in its bytes (lossy
    /// UTF-8) or the deadline elapses. Returns the captured contents.
    ///
    /// We capture what the PTY *child* actually received on stdin (via
    /// `cat > file`) rather than reading the renderer event stream: the
    /// PTY line-discipline ECHO would otherwise mix a second, racing copy
    /// of the input into the output, which tests the wrong direction.
    async fn read_capture_until(path: &std::path::Path, needles: &[&str]) -> String {
        for _ in 0..200 {
            if let Ok(bytes) = std::fs::read(path) {
                let s = String::from_utf8_lossy(&bytes);
                if needles.iter().all(|n| s.contains(n)) {
                    return s.into_owned();
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        std::fs::read(path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    /// Assert each marker appears in `haystack` in the given order.
    fn assert_marker_order(haystack: &str, markers: &[&str]) {
        let mut last: Option<usize> = None;
        for m in markers {
            let pos = haystack.find(m);
            assert!(
                pos.is_some(),
                "marker {m} missing from capture: {haystack:?}"
            );
            if let (Some(prev), Some(cur)) = (last, pos) {
                assert!(
                    prev < cur,
                    "markers out of order at {m}: prev {prev} >= cur {cur} in {haystack:?}"
                );
            }
            last = pos;
        }
    }

    /// Read-only key lookup must never create a session, must find a
    /// live one, and must read as `None` once that session is gone.
    /// Backs the spawn-free `lookup_terminal_session` IPC (tsk139).
    #[tokio::test]
    async fn session_id_for_key_reads_without_spawning() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let key = "s-1|thr3|claude|working".to_string();
        // Nothing registered yet — a read must never spawn or create.
        assert_eq!(reg.session_id_for_key(&key).await, None);

        // Register a session under the key via the normal attach path.
        let dir = std::env::temp_dir();
        let result = reg
            .attach_or_create(key.clone(), 80, 24, |c, r| SpawnRequest {
                command: "cat".into(),
                args: vec![],
                cwd: dir,
                env: vec![],
                env_remove: vec![],
                cols: c,
                rows: r,
            })
            .await
            .expect("spawn session");

        // The read-only lookup finds the live id; an unknown key is None.
        assert_eq!(
            reg.session_id_for_key(&key).await.as_deref(),
            Some(result.session_id.as_str())
        );
        assert_eq!(
            reg.session_id_for_key("s-1|thr3|claude|talking").await,
            None
        );

        // Once killed, the key reads as None again (no stale id leaks).
        let _ = reg.close(&result.session_id).await;
        assert_eq!(reg.session_id_for_key(&key).await, None);
    }

    /// Reattaching to a live session takes the new pane's size. A pane
    /// reopened after an app restart measures its own cols/rows; a process
    /// left at the old size draws its bottom lines past the pane and over
    /// each other.
    #[tokio::test]
    async fn reattaching_resizes_the_live_session_to_the_new_pane() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("size.txt");
        let spawn = |c, r| SpawnRequest {
            command: "sh".into(),
            args: vec![],
            cwd: std::env::temp_dir(),
            env: vec![],
            env_remove: vec![],
            cols: c,
            rows: r,
        };
        let key = "session|ses1".to_string();
        let first = reg
            .attach_or_create(key.clone(), 80, 24, spawn)
            .await
            .expect("spawn session");
        let again = reg
            .attach_or_create(key, 132, 41, spawn)
            .await
            .expect("reattach");
        assert_eq!(again.session_id, first.session_id);

        let command = format!(
            "stty size > {}; echo done >> {}\n",
            capture.display(),
            capture.display()
        );
        reg.send(&first.session_id, &input_message(command.as_bytes()))
            .await
            .unwrap();
        let size = read_capture_until(&capture, &["done"]).await;
        let _ = reg.close(&first.session_id).await;
        assert!(
            size.starts_with("41 132"),
            "the pty kept its old size: {size:?}"
        );
    }

    /// An agent's process exiting ends its session: the registry records a
    /// `SessionEnd` for its agent session (Codex posts none of its own).
    #[tokio::test]
    async fn an_agent_panes_exit_ends_its_session() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let session = svc
            .agent_session_store
            .newest_for_thread(fx.thread)
            .await
            .unwrap()
            .unwrap()
            .id;
        svc.db
            .transaction(move |tx| {
                oxplow_db::agent_session_store::bind_tx(
                    tx,
                    session,
                    "h1",
                    oxplow_domain::agent_session::BindingOrigin::Started,
                    Timestamp::now(),
                )
            })
            .await
            .unwrap();
        svc.terminal_sessions
            .attach_or_create_for_agent(
                "s|ses".to_string(),
                Some(AgentPane {
                    thread: fx.thread,
                    session: Some(session),
                    launch: Default::default(),
                }),
                80,
                24,
                |c, r| SpawnRequest {
                    command: "sh".into(),
                    args: vec!["-c".into(), "exit 0".into()],
                    cwd: std::env::temp_dir(),
                    env: vec![],
                    env_remove: vec![],
                    cols: c,
                    rows: r,
                },
            )
            .await
            .expect("spawn");
        let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let events = svc.event_log_store.read_after(0, 1000).await.unwrap();
                if let Some(e) = events
                    .into_iter()
                    .find(|e| e.envelope.event_type == "agent.session.ended")
                {
                    return e;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the exit is recorded");
        assert_eq!(ended.envelope.payload["reason"], "exit");
        assert_eq!(ended.envelope.payload["session"], "h1");
        assert_eq!(ended.envelope.anchors.agent_session_id, Some(session));
    }

    /// The pid the daemon spawned for an agent session is recorded on it.
    #[tokio::test]
    async fn an_agent_panes_pid_is_recorded_on_its_session() {
        use oxplow_domain::stores::AgentSessionStore as _;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let session = svc
            .agent_session_store
            .newest_for_thread(fx.thread)
            .await
            .unwrap()
            .unwrap()
            .id;
        let result = svc
            .terminal_sessions
            .attach_or_create_for_agent(
                "s|ses-pid".to_string(),
                Some(AgentPane {
                    thread: fx.thread,
                    session: Some(session),
                    launch: Default::default(),
                }),
                80,
                24,
                |c, r| SpawnRequest {
                    command: "sleep".into(),
                    args: vec!["30".into()],
                    cwd: std::env::temp_dir(),
                    env: vec![],
                    env_remove: vec![],
                    cols: c,
                    rows: r,
                },
            )
            .await
            .expect("spawn");
        let pid = svc.terminal_sessions.pid_of(&result.session_id).await;
        let row = svc
            .agent_session_store
            .get(&session)
            .await
            .unwrap()
            .unwrap();
        assert!(pid.is_some());
        assert_eq!(row.pid, pid);
        assert!(row.last_seen_at.is_some());
        let _ = svc.terminal_sessions.close(&result.session_id).await;
    }

    #[test]
    fn an_agent_pane_is_keyed_by_its_session() {
        assert_eq!(
            agent_pane_key(oxplow_domain::AgentSessionId::new(3)),
            "session|ses3"
        );
    }

    /// tsk1026: a session whose process exited is unregistered, so
    /// attaching to its key again starts a fresh one (an agent the person
    /// quit, opened again) instead of replaying the dead one forever.
    #[tokio::test]
    async fn an_exited_session_is_unregistered_and_attaching_starts_a_new_one() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let mut events = reg.subscribe();
        let key = "s-1|thr3|claude|working".to_string();
        let exiting = |c, r| SpawnRequest {
            command: "sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            cwd: std::env::temp_dir(),
            env: vec![],
            env_remove: vec![],
            cols: c,
            rows: r,
        };
        let first = reg
            .attach_or_create(key.clone(), 80, 24, exiting)
            .await
            .expect("spawn session");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let e = events.recv().await.unwrap();
                if e.session_id == first.session_id && e.message.contains("\"exit\"") {
                    break;
                }
            }
        })
        .await
        .expect("the session exits");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while reg.session_id_for_key(&key).await.is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("an exited session is unregistered");
        let second = reg
            .attach_or_create(key.clone(), 80, 24, exiting)
            .await
            .expect("spawn again");
        assert_ne!(second.session_id, first.session_id);
    }

    /// Spawn a `cat > <capture-file>` session and return (registry,
    /// session_id, capture_path). The capture file records the exact
    /// byte stream the child read from its PTY stdin, in order.
    async fn spawn_capture(label: &str) -> (TerminalSessionRegistry, String, std::path::PathBuf) {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let (req, path) = capture_request(label);
        let key = format!("shell:{}", uuid::Uuid::new_v4().simple());
        let session_id = reg
            .spawn_with(req, key, None)
            .await
            .expect("spawn capture shell");
        (reg, session_id, path)
    }

    /// A `cat > <file>` PTY's request: what reaches its stdin lands in the
    /// file, in order.
    fn capture_request(label: &str) -> (SpawnRequest, std::path::PathBuf) {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "oxplow-paste-{label}-{}.capture",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let command = format!("cat > '{}'", path.display());
        let req = SpawnRequest {
            command: "sh".into(),
            args: vec!["-lc".into(), command],
            cwd: dir.clone(),
            env: crate::agent_path::base_pty_env(),
            env_remove: vec![],
            cols: 80,
            rows: 24,
        };
        (req, path)
    }

    /// Input written for a terminal before its PTY exists — typed while it
    /// was still spawning — is held, then arrives once the PTY does, in
    /// order.
    #[tokio::test]
    async fn input_written_before_the_session_exists_arrives_once_it_does() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let key = format!("shell:{}", uuid::Uuid::new_v4().simple());
        for marker in ["EARLY_ONE ", "EARLY_TWO ", "EARLY_THREE\r"] {
            reg.enqueue_input(key.clone(), input_message(marker.as_bytes()));
        }
        let (req, path) = capture_request("early");
        let session_id = reg.spawn_with(req, key, None).await.expect("spawn");
        let captured = read_capture_until(&path, &["EARLY_ONE", "EARLY_TWO", "EARLY_THREE"]).await;
        assert_marker_order(&captured, &["EARLY_ONE", "EARLY_TWO", "EARLY_THREE"]);
        let _ = reg.close(&session_id).await;
        let _ = std::fs::remove_file(&path);
    }

    /// Many small writes, each its own message (a fast typist's keys),
    /// reach the PTY in the order they were written.
    #[tokio::test]
    async fn many_inputs_arrive_in_order() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let key = format!("shell:{}", uuid::Uuid::new_v4().simple());
        let (req, path) = capture_request("many");
        let session_id = reg.spawn_with(req, key.clone(), None).await.expect("spawn");
        let markers: Vec<String> = (0..200).map(|i| format!("K{i:03} ")).collect();
        for m in &markers {
            reg.enqueue_input(key.clone(), input_message(m.as_bytes()));
        }
        reg.enqueue_input(key.clone(), input_message(b"\r"));
        let needles: Vec<&str> = markers.iter().map(|m| m.trim()).collect();
        let captured = read_capture_until(&path, &needles).await;
        assert_marker_order(&captured, &needles);
        let _ = reg.close(&session_id).await;
        let _ = std::fs::remove_file(&path);
    }

    /// Input for a terminal whose PTY never comes (its launch failed) is
    /// dropped after the wait, never replayed into a later process.
    #[tokio::test]
    async fn input_for_a_key_that_never_opens_is_dropped() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        )
        .with_input_wait(std::time::Duration::from_millis(100));
        let key = format!("shell:{}", uuid::Uuid::new_v4().simple());
        reg.enqueue_input(key.clone(), input_message(b"STALE "));
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let (req, path) = capture_request("stale");
        let session_id = reg.spawn_with(req, key.clone(), None).await.expect("spawn");
        reg.enqueue_input(key.clone(), input_message(b"FRESH\r"));
        let captured = read_capture_until(&path, &["FRESH"]).await;
        assert!(!captured.contains("STALE"), "{captured:?}");
        let _ = reg.close(&session_id).await;
        let _ = std::fs::remove_file(&path);
    }

    /// Regression for tsk93: a multi-paragraph paste (blank-line
    /// separated, framed as a single bracketed paste the way xterm.js
    /// emits it) must reach the PTY child as one contiguous, in-order
    /// byte sequence — never scrambled. Guards the web→PTY write path
    /// (`send` → one `pty.write`) against chunk-reordering regressions.
    #[tokio::test]
    async fn multi_paragraph_paste_reaches_pty_in_order() {
        let (reg, session_id, path) = spawn_capture("small").await;

        // The exact shape xterm.js produces for a 3-paragraph paste with
        // bracketed-paste mode on: ESC[200~ <text, \n→\r normalized> ESC[201~.
        let payload = b"\x1b[200~PARA_ALPHA\r\rPARA_BRAVO\r\rPARA_CHARLIE\r\x1b[201~";
        reg.send(&session_id, &input_message(payload))
            .await
            .expect("send paste");

        let captured =
            read_capture_until(&path, &["PARA_ALPHA", "PARA_BRAVO", "PARA_CHARLIE"]).await;
        assert_marker_order(&captured, &["PARA_ALPHA", "PARA_BRAVO", "PARA_CHARLIE"]);

        let _ = reg.close(&session_id).await;
        let _ = std::fs::remove_file(&path);
    }

    /// Larger-scale variant: a paste big enough to span multiple PTY
    /// reads must still arrive with its ordered markers in order. This
    /// is the regime the dogfooded bug was reported at (~1.2 KB, three
    /// paragraphs) — it catches a reassembly/chunk-reorder regression a
    /// tiny single-read payload could miss.
    #[tokio::test]
    async fn large_multi_chunk_paste_preserves_marker_order() {
        let (reg, session_id, path) = spawn_capture("large").await;

        // 24 ordered markers separated by filler + blank lines, wrapped
        // as one bracketed paste — well over a single small read.
        let markers: Vec<String> = (0..24).map(|i| format!("MK{i:02}")).collect();
        let mut body = String::new();
        for (i, m) in markers.iter().enumerate() {
            if i > 0 {
                body.push_str("\r\r");
            }
            body.push_str(m);
            body.push_str(" lorem ipsum dolor sit amet consectetur adipiscing");
        }
        let payload = format!("\x1b[200~{body}\r\x1b[201~");
        reg.send(&session_id, &input_message(payload.as_bytes()))
            .await
            .expect("send large paste");

        let needles: Vec<&str> = markers.iter().map(|s| s.as_str()).collect();
        let captured = read_capture_until(&path, &needles).await;
        assert_marker_order(&captured, &needles);

        let _ = reg.close(&session_id).await;
        let _ = std::fs::remove_file(&path);
    }

    /// The daemon's exit ends every session's process: each leads its own
    /// group, outside the daemon's, so nothing else would.
    #[cfg(unix)]
    #[tokio::test]
    async fn close_all_ends_every_session() {
        let reg = TerminalSessionRegistry::new(
            PtyManager::spawn(),
            crate::output_activity::OutputActivity::new(),
        );
        let deaf = |c, r| SpawnRequest {
            command: "sh".into(),
            args: vec!["-c".into(), "trap '' HUP TERM; exec sleep 60".into()],
            cwd: std::env::temp_dir(),
            env: vec![],
            env_remove: vec![],
            cols: c,
            rows: r,
        };
        let mut pids = Vec::new();
        for key in ["session|ses1", "terminal|trm1"] {
            let s = reg
                .attach_or_create(key.to_string(), 80, 24, deaf)
                .await
                .expect("spawn session");
            pids.push(reg.pid_of(&s.session_id).await.expect("a pid") as i32);
        }

        reg.close_all().await;

        for key in ["session|ses1", "terminal|trm1"] {
            assert_eq!(
                reg.session_id_for_key(key).await,
                None,
                "{key} still registered"
            );
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let gone = |pid: i32| unsafe { libc::kill(pid, 0) } != 0;
        while !pids.iter().all(|p| gone(*p)) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let left: Vec<_> = pids.into_iter().filter(|p| !gone(*p)).collect();
        for pid in &left {
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
        assert!(left.is_empty(), "sessions outlived close_all: {left:?}");
    }
}
