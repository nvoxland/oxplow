//! PTY management via `portable-pty`, owner-task pattern.
//!
//! A single `PtyManager` task owns the registry of spawned panes;
//! callers send `mpsc` messages and receive replies via `oneshot`.
//! Bytes flow out via `broadcast` channels so multiple subscribers
//! (e.g. several open webviews of the same pane) get the same
//! stream without duplication.
//!
//! Windows reliability: there's a documented `portable-pty` race
//! where `SlavePty` outliving `MasterPty` causes a native assertion
//! failure during teardown. We mitigate by wrapping the master in
//! `Option<…>` and explicitly dropping it before the slave goes
//! away (see `PaneEntry::Drop`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("pane not found: {0}")]
    NotFound(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("pty io: {0}")]
    Io(#[from] std::io::Error),
    #[error("manager shutting down")]
    Shutdown,
}

/// Unique identifier for a spawned PTY pane.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct PaneId(pub String);

impl Default for PaneId {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneId {
    pub fn new() -> Self {
        Self(format!("pane-{}", uuid::Uuid::new_v4().simple()))
    }
}

/// Spawn parameters.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// What a pane emits.
#[derive(Debug, Clone)]
pub enum PaneEvent {
    /// Bytes read from the master.
    Output(Bytes),
    /// Process exited; the pane is gone after this.
    Exit { exit_code: Option<i32> },
}

/// Public handle to the manager. Cloneable; commands are sent to the
/// owner task via `mpsc`.
#[derive(Clone)]
pub struct PtyManager {
    cmd_tx: mpsc::Sender<Cmd>,
}

impl PtyManager {
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        tokio::spawn(owner_loop(cmd_rx));
        Self { cmd_tx }
    }

    pub async fn spawn_pane(&self, req: SpawnRequest) -> Result<PaneHandle, PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Spawn {
                req,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn write(&self, id: &PaneId, bytes: Bytes) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Write {
                id: id.clone(),
                bytes,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn resize(&self, id: &PaneId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Resize {
                id: id.clone(),
                cols,
                rows,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn kill(&self, id: &PaneId) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Kill {
                id: id.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }
}

/// What `spawn_pane` returns: an id plus a subscriber so the caller
/// doesn't race the first burst of output.
pub struct PaneHandle {
    pub id: PaneId,
    pub events: broadcast::Receiver<PaneEvent>,
    /// OS process id of the spawned child, when the platform reported
    /// one. Used to read the live cwd (terminal file-link resolution).
    pub pid: Option<u32>,
}

/// Internal command messages sent to the owner task.
enum Cmd {
    Spawn {
        req: SpawnRequest,
        reply: oneshot::Sender<Result<PaneHandle, PtyError>>,
    },
    Write {
        id: PaneId,
        bytes: Bytes,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
    Resize {
        id: PaneId,
        cols: u16,
        rows: u16,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
    Kill {
        id: PaneId,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
}

/// State for a single live pane held inside the owner task.
struct PaneEntry {
    /// Wrapped in `Option` so we can `take()` it during Drop and force
    /// synchronous cleanup on Windows where SlavePty outliving
    /// MasterPty triggers a native assertion (see `portable-pty`
    /// teardown race).
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn std::io::Write + Send>,
    /// Kept around so subscribers added after spawn still get events.
    /// Dead-code-warning suppressed: it's load-bearing via clones.
    #[allow(dead_code)]
    sender: broadcast::Sender<PaneEvent>,
    /// Shared with the wait task; whichever path (Kill or natural
    /// exit) gets there first owns the child and handles cleanup. The
    /// other path sees `None` and no-ops.
    child: Arc<std::sync::Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    /// PID stashed at spawn time so Kill can signal the process even
    /// when the wait task is currently holding the Child handle. Some
    /// processes (e.g. `sleep`) ignore SIGHUP from a closed PTY, so
    /// we can't rely on master-drop alone to terminate.
    pid: Option<u32>,
}

impl Drop for PaneEntry {
    fn drop(&mut self) {
        // Take the master FIRST so it's freed before the child + slave
        // bookkeeping runs. This is the core of the Windows ConPTY
        // race mitigation.
        let _ = self.master.take();
        if let Some(mut child) = self.child.lock().ok().and_then(|mut g| g.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn owner_loop(mut cmd_rx: mpsc::Receiver<Cmd>) {
    let mut panes: HashMap<PaneId, PaneEntry> = HashMap::new();

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Spawn { req, reply } => {
                let _ = reply.send(spawn_one(&mut panes, req).await);
            }
            Cmd::Write { id, bytes, reply } => {
                let result = match panes.get_mut(&id) {
                    Some(entry) => entry
                        .writer
                        .write_all(&bytes)
                        .map_err(PtyError::from)
                        .and_then(|_| entry.writer.flush().map_err(PtyError::from)),
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
            Cmd::Resize {
                id,
                cols,
                rows,
                reply,
            } => {
                let result = match panes.get(&id) {
                    Some(entry) => entry
                        .master
                        .as_ref()
                        .ok_or(PtyError::NotFound(id.0.clone()))
                        .and_then(|m| {
                            m.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            })
                            .map_err(|e| PtyError::Spawn(e.to_string()))
                        }),
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
            Cmd::Kill { id, reply } => {
                let result = match panes.remove(&id) {
                    Some(entry) => {
                        // Two paths to ensure the process dies:
                        //
                        // 1. If the entry still holds the Child handle
                        //    (no natural exit raced us), use the
                        //    portable_pty Child::kill, which sends
                        //    SIGKILL on Unix.
                        // 2. Otherwise the wait task is blocking on
                        //    child.wait(); send SIGTERM directly to
                        //    the PID so wait() returns. This covers
                        //    processes that ignore SIGHUP from a
                        //    closed PTY (e.g. `sleep`).
                        let child_taken = entry.child.lock().ok().and_then(|mut g| g.take());
                        if let Some(mut child) = child_taken {
                            let _ = child.kill();
                            let _ = child.wait();
                        } else if let Some(pid) = entry.pid {
                            kill_pid(pid);
                        }
                        // entry's Drop runs at end of scope and frees
                        // the master, completing the Windows-safe
                        // teardown order.
                        drop(entry);
                        Ok(())
                    }
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
        }
    }

    debug!("pty manager shutting down; releasing {} panes", panes.len());
    panes.clear();
}

/// Best-effort process termination by PID. Unix sends SIGKILL via
/// `libc::kill`; Windows uses `TerminateProcess` via the win32 API.
/// Used as a fallback when the wait task already owns the
/// `portable_pty::Child` handle and Kill can't reach it directly.
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

async fn spawn_one(
    panes: &mut HashMap<PaneId, PaneEntry>,
    req: SpawnRequest,
) -> Result<PaneHandle, PtyError> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: req.rows,
            cols: req.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| PtyError::Spawn(e.to_string()))?;

    let mut cmd = CommandBuilder::new(&req.command);
    cmd.args(req.args.iter().map(|s| s.as_str()));
    cmd.cwd(&req.cwd);
    for (k, v) in req.env.iter() {
        cmd.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| PtyError::Spawn(e.to_string()))?;
    let pid = child.process_id();

    // Slave end can be dropped now that the child has it.
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| PtyError::Spawn(e.to_string()))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| PtyError::Spawn(e.to_string()))?;

    let (sender, _) = broadcast::channel::<PaneEvent>(1024);
    let id = PaneId::new();

    // Shared child handle: lives in the entry (so Kill can call
    // `child.kill()`) AND in the wait task (so it can call `wait()`
    // to get the exit code). Both paths take it out of the Option
    // when they need exclusive access; whichever wins handles
    // cleanup, the other no-ops.
    let child_handle: Arc<std::sync::Mutex<Option<Box<dyn Child + Send + Sync>>>> =
        Arc::new(std::sync::Mutex::new(Some(child)));

    // Reader task: blocking read in a spawn_blocking, push to broadcast.
    {
        let sender = sender.clone();
        let id_dbg = id.clone();
        tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if sender
                            .send(PaneEvent::Output(Bytes::copy_from_slice(&buf[..n])))
                            .is_err()
                        {
                            // No subscribers — keep reading anyway so the
                            // child doesn't block on a full pipe.
                        }
                    }
                    Err(e) => {
                        warn!(pane = %id_dbg.0, "pty read error: {e}");
                        break;
                    }
                }
            }
        });
    }

    // Wait task: on child exit, emit Exit. Takes the child out of the
    // shared Option; if Kill already took it, this no-ops. We use
    // spawn_blocking because portable_pty::Child::wait is synchronous.
    {
        let exit_sender = sender.clone();
        let wait_handle = Arc::clone(&child_handle);
        tokio::task::spawn_blocking(move || {
            let mut child = match wait_handle.lock().ok().and_then(|mut g| g.take()) {
                Some(c) => c,
                None => return,
            };
            let exit_code = child.wait().ok().map(|s| s.exit_code() as i32);
            let _ = exit_sender.send(PaneEvent::Exit { exit_code });
        });
    }

    let events = sender.subscribe();

    panes.insert(
        id.clone(),
        PaneEntry {
            master: Some(pair.master),
            writer,
            sender,
            child: child_handle,
            pid,
        },
    );

    Ok(PaneHandle { id, events, pid })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    fn cat_command() -> SpawnRequest {
        SpawnRequest {
            command: "cat".into(),
            args: vec![],
            cwd: tempdir().unwrap().keep(),
            env: vec![],
            cols: 80,
            rows: 24,
        }
    }

    #[tokio::test]
    async fn spawn_and_kill() {
        let mgr = PtyManager::spawn();
        let req = SpawnRequest {
            command: "sleep".into(),
            args: vec!["10".into()],
            cwd: tempdir().unwrap().keep(),
            env: vec![],
            cols: 80,
            rows: 24,
        };
        let handle = mgr.spawn_pane(req).await.unwrap();
        mgr.kill(&handle.id).await.unwrap();
        // After kill, subsequent operations on the same id return NotFound.
        let result = mgr.write(&handle.id, Bytes::from_static(b"x")).await;
        assert!(matches!(result, Err(PtyError::NotFound(_))));
    }

    #[tokio::test]
    async fn write_echoes_back_through_cat() {
        let mgr = PtyManager::spawn();
        let mut handle = mgr.spawn_pane(cat_command()).await.unwrap();

        // Drain any startup output the OS may have buffered so we can
        // then look for our own bytes coming back.
        let _ = timeout(Duration::from_millis(100), handle.events.recv()).await;

        mgr.write(&handle.id, Bytes::from_static(b"hello\n"))
            .await
            .unwrap();

        // Read until we see "hello" in the stream (cat's PTY echo
        // turns each input byte into output).
        let mut got = Vec::new();
        for _ in 0..50 {
            match timeout(Duration::from_millis(200), handle.events.recv()).await {
                Ok(Ok(PaneEvent::Output(b))) => {
                    got.extend_from_slice(&b);
                    if got.windows(5).any(|w| w == b"hello") {
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            got.windows(5).any(|w| w == b"hello"),
            "expected echo, got {:?}",
            String::from_utf8_lossy(&got)
        );

        mgr.kill(&handle.id).await.unwrap();
    }

    /// Regression test for the kill-doesn't-actually-kill bug found
    /// in code review: the kill command must terminate the spawned
    /// process, not just remove the registry entry. We verify by
    /// spawning a long-running `sleep`, killing it, and observing an
    /// `Exit` event arrives quickly.
    #[tokio::test]
    async fn kill_terminates_running_child() {
        let mgr = PtyManager::spawn();
        let req = SpawnRequest {
            command: "sleep".into(),
            args: vec!["60".into()],
            cwd: tempdir().unwrap().keep(),
            env: vec![],
            cols: 80,
            rows: 24,
        };
        let mut handle = mgr.spawn_pane(req).await.unwrap();
        // Wait for the spawn to settle.
        let _ = timeout(Duration::from_millis(100), handle.events.recv()).await;

        mgr.kill(&handle.id).await.unwrap();

        // The wait task should observe the killed child and emit Exit
        // within a reasonable window. If kill silently failed, we'd
        // wait the full 60s for sleep to finish naturally.
        let exit = timeout(Duration::from_secs(5), async {
            loop {
                match handle.events.recv().await {
                    Ok(PaneEvent::Exit { .. }) => return Ok::<(), ()>(()),
                    Ok(_) => continue, // skip output bytes
                    Err(_) => return Err(()),
                }
            }
        })
        .await;
        assert!(exit.is_ok(), "child should exit promptly after kill");
    }

    #[tokio::test]
    async fn resize_unknown_pane_errors() {
        let mgr = PtyManager::spawn();
        let id = PaneId::new();
        let result = mgr.resize(&id, 100, 30).await;
        assert!(matches!(result, Err(PtyError::NotFound(_))));
    }
}
