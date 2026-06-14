//! Terminal session registry powering the renderer's `TerminalPane`.
//!
//! Each session bridges an xterm.js instance in the renderer to a
//! tmux pane on the host. The renderer talks the same JSON protocol
//! the original Electron build used:
//!
//! - Outgoing (renderer → daemon):
//!   - `{type:"input", bytes:base64}` — user keystrokes
//!   - `{type:"input-binary", bytes:base64}` — binary input (paste)
//!   - `{type:"resize", cols, rows}` — viewport changed
//!   - `{type:"history-page", direction:"up"|"down"}` — page in copy-mode
//!   - `{type:"history-scroll", lines:int}` — scroll N lines (positive = older)
//!   - `{type:"history-exit"}` — leave copy-mode
//!
//! - Incoming (daemon → renderer):
//!   - `{type:"data", bytes:base64}` — bytes from the PTY
//!
//! Implementation: spawn `tmux attach-session -t <pane_target>` via
//! `oxplow_pty::PtyManager`. PTY bytes flow back as `data` events.
//! Resize and copy-mode messages dispatch to the shared `TmuxRunner`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
pub use oxplow_pty::SpawnRequest;
use oxplow_pty::{PaneEvent, PaneId, PtyManager};
use oxplow_tmux::{ScrollDirection, TmuxRunner, WindowTarget};
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
    pane_target: String,
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

#[derive(Clone)]
pub struct TerminalSessionRegistry {
    pty: PtyManager,
    tmux: Arc<dyn TmuxRunner>,
    inner: Arc<Mutex<HashMap<String, SessionEntry>>>,
    /// External-key → session_id index. Lets `attach_or_create`
    /// look up an existing session for a given (stream, thread,
    /// pane, transport) tuple in O(1).
    by_key: Arc<Mutex<HashMap<SessionKey, String>>>,
    events_tx: broadcast::Sender<TerminalBridgeEvent>,
}

impl TerminalSessionRegistry {
    pub fn new(pty: PtyManager, tmux: Arc<dyn TmuxRunner>) -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            pty,
            tmux,
            inner: Arc::new(Mutex::new(HashMap::new())),
            by_key: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
        }
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
        pane_target: String,
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
        let session_id = self.spawn_with(pane_target, req, key.clone()).await?;
        Ok(Self::build_attach_result(session_id, Vec::new()))
    }

    /// Open a new session. Spawns `tmux attach-session -t <pane_target>`
    /// via the PtyManager and starts forwarding bytes back as `data`
    /// messages. Returns the new session_id.
    ///
    /// Used directly only by tests / older callers; production paths
    /// should go through `attach_or_create`.
    pub async fn open(
        &self,
        pane_target: String,
        cols: u16,
        rows: u16,
    ) -> Result<String, TerminalSessionError> {
        let req = SpawnRequest {
            command: "tmux".into(),
            args: vec!["attach-session".into(), "-t".into(), pane_target.clone()],
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ],
            cols,
            rows,
        };
        let key = format!("legacy:{}", uuid::Uuid::new_v4().simple());
        self.spawn_with(pane_target, req, key).await
    }

    /// Direct-mode open: spawn a shell command in a fresh PTY (no
    /// tmux). Mirrors the main-branch `AgentPty` path — useful when
    /// the renderer is wired directly to the agent CLI without going
    /// through tmux. `pane_target` is stored as a label only; the
    /// underlying PTY is the spawned `sh -lc <command>`.
    pub async fn open_command(
        &self,
        pane_target: String,
        command: String,
        cwd: std::path::PathBuf,
        cols: u16,
        rows: u16,
    ) -> Result<String, TerminalSessionError> {
        let req = SpawnRequest {
            command: "sh".into(),
            args: vec!["-lc".into(), command],
            cwd,
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ],
            cols,
            rows,
        };
        let key = format!("legacy:{}", uuid::Uuid::new_v4().simple());
        self.spawn_with(pane_target, req, key).await
    }

    async fn spawn_with(
        &self,
        pane_target: String,
        req: SpawnRequest,
        key: SessionKey,
    ) -> Result<String, TerminalSessionError> {
        let mut handle = self.pty.spawn_pane(req).await?;
        let pane_id = handle.id.clone();
        let pid = handle.pid;
        let session_id = format!("term-{}", uuid::Uuid::new_v4().simple());

        // Force a tmux repaint so freshly-attached clients see the
        // current pane state immediately even if no new output is
        // produced.
        self.tmux.refresh_clients().await;

        // Spawn a forwarder that pumps PaneEvents → TerminalBridgeEvents
        // and tees a copy into the session's replay buffer so any
        // future re-attach starts from the same screen state.
        let session_id_for_task = session_id.clone();
        let events = self.events_tx.clone();
        let ring = Arc::new(Mutex::new(RingBuffer::new()));
        let ring_for_task = Arc::clone(&ring);
        let forwarder = tokio::spawn(async move {
            loop {
                match handle.events.recv().await {
                    Ok(PaneEvent::Output(bytes)) => {
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

        self.inner.lock().await.insert(
            session_id.clone(),
            SessionEntry {
                pane_target,
                pane_id,
                pid,
                forwarder,
                ring,
                key: key.clone(),
            },
        );
        self.by_key.lock().await.insert(key, session_id.clone());
        Ok(session_id)
    }

    /// Best-effort live working directory of a session's child process.
    /// Meaningful for the direct `shell` pane (the child IS the shell, so
    /// this tracks `cd`); for tmux-backed panes the child is the tmux client,
    /// so it reflects the worktree root, not the active pane's shell.
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

        let (pane_id, pane_target) = {
            let map = self.inner.lock().await;
            let entry = map
                .get(session_id)
                .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
            (entry.pane_id.clone(), entry.pane_target.clone())
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
                // Reject absurdly-small resizes — see history-mode comment
                // in the original Electron pty-bridge: a hidden xterm can
                // fit-down to two cells and shrink the real tmux window.
                if cols < 20 || rows < 5 {
                    return Ok(());
                }
                self.pty.resize(&pane_id, cols, rows).await?;
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.resize_window(&target, cols, rows).await;
                }
            }
            "history-page" => {
                let dir = match parsed.get("direction").and_then(|v| v.as_str()) {
                    Some("up") => ScrollDirection::Up,
                    Some("down") => ScrollDirection::Down,
                    _ => return Ok(()),
                };
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.copy_mode_page(&target, dir).await;
                }
            }
            "history-scroll" => {
                let lines = parsed.get("lines").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.copy_mode_scroll(&target, lines).await;
                }
            }
            "history-exit" => {
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.exit_copy_mode(&target).await;
                }
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

/// Pane targets are `"<session>:<window>"`; both sides non-empty.
fn parse_window_target(pane_target: &str) -> Option<WindowTarget> {
    let (session, window) = pane_target.split_once(':')?;
    if session.is_empty() || window.is_empty() {
        return None;
    }
    let session = oxplow_tmux::Session(session.to_string());
    Some(WindowTarget::from_parts(&session, window))
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

    #[test]
    fn parse_window_target_round_trip() {
        let t = parse_window_target("oxplow-foo:work").unwrap();
        assert_eq!(t.as_str(), "oxplow-foo:work");
    }

    #[test]
    fn parse_window_target_rejects_malformed() {
        assert!(parse_window_target("nopeartf").is_none());
        assert!(parse_window_target(":x").is_none());
        assert!(parse_window_target("x:").is_none());
    }

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

    /// Spawn a `cat > <capture-file>` session and return (registry,
    /// session_id, capture_path). The capture file records the exact
    /// byte stream the child read from its PTY stdin, in order.
    async fn spawn_capture(label: &str) -> (TerminalSessionRegistry, String, std::path::PathBuf) {
        let reg =
            TerminalSessionRegistry::new(PtyManager::spawn(), Arc::new(oxplow_tmux::SystemTmux));
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "oxplow-paste-{label}-{}.capture",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let command = format!("cat > '{}'", path.display());
        let session_id = reg
            .open_command(format!("test-{label}"), command, dir, 80, 24)
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
