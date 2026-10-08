//! Terminal session registry powering the renderer's `TerminalPane`.
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
use thiserror::Error;
use tokio::sync::{broadcast, Mutex};
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
/// Forwarded to the renderer over `terminal:event`.
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

/// The agent a PTY runs: its thread and agent session. Its output stamps
/// the session's liveness, and its exit ends the session's harness session
/// (`HookIngestService` records a `SessionEnd` for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentPane {
    pub thread: ThreadId,
    pub session: Option<oxplow_domain::AgentSessionId>,
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
}

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
        // Fast path: existing session for this key — replay its buffer.
        if let Some(existing_id) = self.by_key.lock().await.get(&key).cloned() {
            let map = self.inner.lock().await;
            if let Some(entry) = map.get(&existing_id) {
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
    /// thread's agent PTY to `forward_terminal_input` without going
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
                        let msg = serde_json::json!({
                            "type": "exit",
                            "exitCode": exit_code,
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
                        }
                        drop(keys);
                        if let (Some(pane), Some(ingest)) = (agent, exits.get()) {
                            let exit = crate::hook_ingest::HookEnvelope {
                                kind: oxplow_domain::HookKind::SessionEnd,
                                thread_id: Some(pane.thread),
                                stream_id: None,
                                agent_session_id: pane.session,
                                session_id: None,
                                payload_json: serde_json::json!({ "reason": "exit" }).to_string(),
                                prompt: None,
                                decision: None,
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
        Ok(session_id)
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

    /// Dispatch one renderer-issued JSON message.
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
    pub async fn close(&self, session_id: &str) -> Result<(), TerminalSessionError> {
        let mut map = self.inner.lock().await;
        let entry = map
            .remove(session_id)
            .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
        entry.forwarder.abort();
        let _ = self.pty.kill(&entry.pane_id).await;
        self.by_key.lock().await.remove(&entry.key);
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
    use std::time::Duration;

    /// Build an `{type:"input", bytes:<base64>}` message the way the
    /// renderer's `forwardTerminalInput` does for a paste.
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
                oxplow_db::agent_session_store::set_resume_tx(tx, session, "h1", Timestamp::now())
            })
            .await
            .unwrap();
        svc.terminal_sessions
            .attach_or_create_for_agent(
                "s|ses".to_string(),
                Some(AgentPane {
                    thread: fx.thread,
                    session: Some(session),
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
        let key = format!("shell:{}", uuid::Uuid::new_v4().simple());
        let session_id = reg
            .spawn_with(req, key, None)
            .await
            .expect("spawn capture shell");
        (reg, session_id, path)
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
}
