//! Every open ACP session, by thread. Holds only each session's command
//! sender and shared view; the actor (`session.rs`) owns the connection.
//! Events for every session go out on one broadcast channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oxplow_domain::ThreadId;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::host::AcpHost;
use super::model::ContextUsage;
use super::session::{AcpError, AcpEvent, AcpStatus, Actor, Command, SessionSpec, SessionView};
use super::transcript::TranscriptItem;
use super::wire;

/// The program to run for an agent.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Absolute path.
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// A session as the UI reads it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AcpSnapshot {
    pub agent: String,
    /// The session's generation: a new one (a Restart) replaces the
    /// client's transcript instead of merging into it.
    pub generation: u64,
    pub status: AcpStatus,
    pub usage: Option<ContextUsage>,
    /// The highest `seq` in the transcript; ask `since` this next time.
    pub head_seq: u64,
    pub items: Vec<TranscriptItem>,
    pub stderr_tail: Vec<String>,
}

struct Handle {
    commands: mpsc::UnboundedSender<Command>,
    view: Arc<Mutex<SessionView>>,
    generation: u64,
    /// Set the moment `close` is called, before the actor has wound down.
    closed: Arc<AtomicBool>,
    /// Cleared when a newer session replaces this one; its actor then
    /// records nothing on its way out (the new session owns the thread).
    current: Arc<AtomicBool>,
}

impl Handle {
    fn alive(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && !self.commands.is_closed()
    }
}

/// A thread's slot, claimed before anything is spawned (see
/// [`AcpManager::reserve`]).
struct Reservation {
    thread: ThreadId,
    generation: u64,
    commands: mpsc::UnboundedReceiver<Command>,
    view: Arc<Mutex<SessionView>>,
    current: Arc<AtomicBool>,
}

pub struct AcpManager {
    sessions: Mutex<HashMap<ThreadId, Handle>>,
    events: broadcast::Sender<AcpEvent>,
    /// Hands out session generations; one per open.
    next_generation: std::sync::atomic::AtomicU64,
}

impl Default for AcpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AcpManager {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            next_generation: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AcpEvent> {
        self.events.subscribe()
    }

    /// Publish an event as a session would (transport tests).
    pub fn emit_event_for_tests(&self, event: AcpEvent) {
        let _ = self.events.send(event);
    }

    /// Is a session running for `thread`?
    pub fn is_open(&self, thread: &ThreadId) -> bool {
        self.sessions.lock().get(thread).is_some_and(Handle::alive)
    }

    /// Claim `thread`'s slot for a new session, under one lock, before
    /// anything is spawned: two concurrent opens can't both start an
    /// agent. `None` when a session is already live. A closed session
    /// still winding down is replaced and marked not current.
    fn reserve(&self, spec: &SessionSpec) -> Option<Reservation> {
        let mut sessions = self.sessions.lock();
        if let Some(old) = sessions.get(&spec.thread_id) {
            if old.alive() {
                return None;
            }
            old.current.store(false, Ordering::SeqCst);
        }
        let generation = self.generation();
        let (tx, rx) = mpsc::unbounded_channel();
        let view = Arc::new(Mutex::new(SessionView::new(&spec.agent, generation)));
        let current = Arc::new(AtomicBool::new(true));
        sessions.insert(
            spec.thread_id,
            Handle {
                commands: tx,
                view: view.clone(),
                generation,
                closed: Arc::new(AtomicBool::new(false)),
                current: current.clone(),
            },
        );
        Some(Reservation {
            thread: spec.thread_id,
            generation,
            commands: rx,
            view,
            current,
        })
    }

    /// Give up a reservation whose start failed (only if it's still ours).
    fn release(&self, thread: &ThreadId, generation: u64) {
        let mut sessions = self.sessions.lock();
        if sessions
            .get(thread)
            .is_some_and(|h| h.generation == generation)
        {
            sessions.remove(thread);
        }
    }

    /// Start the agent process and its session. A session already running
    /// for the thread is kept (`Ok`).
    pub async fn open(
        &self,
        host: Arc<dyn AcpHost>,
        spec: SessionSpec,
        launch: Launch,
    ) -> Result<(), AcpError> {
        let Some(reservation) = self.reserve(&spec) else {
            return Ok(());
        };
        let mut cmd = tokio::process::Command::new(&launch.program);
        cmd.args(&launch.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for k in crate::agent_path::NOT_INHERITED {
            cmd.env_remove(k);
        }
        if let Some(path) = crate::agent_path::augmented_path() {
            cmd.env("PATH", path);
        }
        for (k, v) in &launch.env {
            cmd.env(k, v);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.release(&reservation.thread, reservation.generation);
                return Err(AcpError::Agent(format!(
                    "starting {}: {e}",
                    launch.program.display()
                )));
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            self.release(&reservation.thread, reservation.generation);
            return Err(AcpError::Agent("the agent's stdio is unavailable".into()));
        };
        if let Some(stderr) = child.stderr.take() {
            let view = reservation.view.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    view.lock().push_stderr(line);
                }
            });
        }
        self.start(host, spec, reservation, stdin, stdout, Some(child))
            .await
    }

    /// Run a session over an existing transport (tests, the in-process
    /// fake). `write` is the agent's input, `read` its output.
    pub async fn open_with_io<W, R>(
        &self,
        host: Arc<dyn AcpHost>,
        spec: SessionSpec,
        write: W,
        read: R,
    ) -> Result<(), AcpError>
    where
        W: AsyncWrite + Send + 'static,
        R: AsyncRead + Send + 'static,
    {
        let Some(reservation) = self.reserve(&spec) else {
            return Ok(());
        };
        self.start(host, spec, reservation, write, read, None).await
    }

    fn generation(&self) -> u64 {
        self.next_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    async fn start<W, R>(
        &self,
        host: Arc<dyn AcpHost>,
        spec: SessionSpec,
        reservation: Reservation,
        write: W,
        read: R,
        child: Option<tokio::process::Child>,
    ) -> Result<(), AcpError>
    where
        W: AsyncWrite + Send + 'static,
        R: AsyncRead + Send + 'static,
    {
        let Reservation {
            commands: cmd_rx,
            view,
            current,
            ..
        } = reservation;
        let (ready_tx, ready_rx) = oneshot::channel();
        let actor = Actor::new(spec, host, view.clone(), self.events.clone(), current);
        let teardown = actor.teardown_handle();
        tokio::spawn(async move {
            let r = wire::run(write, read, move |conn, incoming| {
                actor.run(conn, incoming, cmd_rx, ready_tx)
            })
            .await;
            // A transport error drops the actor mid-loop, before its own
            // teardown; end the session here (a no-op when it already ran).
            let reason = match r {
                Ok(()) => "the agent connection ended".to_string(),
                Err(e) => {
                    tracing::warn!(error = %e, "acp: connection ended with an error");
                    format!("the agent connection failed: {e}")
                }
            };
            teardown.run(Some(reason)).await;
            // Dropping the child kills it (`kill_on_drop`).
            drop(child);
        });
        match ready_rx.await {
            Ok(r) => r,
            Err(_) => {
                let tail = view.lock().stderr_tail.join("\n");
                Err(AcpError::Agent(if tail.is_empty() {
                    "the agent exited during startup".into()
                } else {
                    format!("the agent exited during startup:\n{tail}")
                }))
            }
        }
    }

    fn commands(&self, thread: &ThreadId) -> Result<mpsc::UnboundedSender<Command>, AcpError> {
        self.sessions
            .lock()
            .get(thread)
            .filter(|h| h.alive())
            .map(|h| h.commands.clone())
            .ok_or(AcpError::NotOpen)
    }

    /// Send what a person typed. The only caller is the UI-only RPC
    /// command behind the prompt box (the source guard pins this).
    pub async fn submit_human_prompt(
        &self,
        thread: &ThreadId,
        text: String,
    ) -> Result<(), AcpError> {
        let (reply, rx) = oneshot::channel();
        self.commands(thread)?
            .send(Command::Prompt { text, reply })
            .map_err(|_| AcpError::NotOpen)?;
        rx.await.map_err(|_| AcpError::NotOpen)?
    }

    pub fn cancel(&self, thread: &ThreadId) -> Result<(), AcpError> {
        self.commands(thread)?
            .send(Command::Cancel)
            .map_err(|_| AcpError::NotOpen)
    }

    /// Answer a permission card; `option_id: None` cancels it.
    pub async fn respond_permission(
        &self,
        thread: &ThreadId,
        request_id: String,
        option_id: Option<String>,
    ) -> Result<(), AcpError> {
        let (reply, rx) = oneshot::channel();
        self.commands(thread)?
            .send(Command::Respond {
                request_id,
                option_id,
                reply,
            })
            .map_err(|_| AcpError::NotOpen)?;
        rx.await.map_err(|_| AcpError::NotOpen)?
    }

    /// The session's state and the items changed after `since`. Also works
    /// for a session that has stopped (its transcript stays readable).
    pub fn transcript(&self, thread: &ThreadId, since: u64) -> Option<AcpSnapshot> {
        let sessions = self.sessions.lock();
        let v = sessions.get(thread)?.view.lock();
        Some(AcpSnapshot {
            agent: v.agent.clone(),
            generation: v.generation,
            status: v.status,
            usage: v.transcript.usage().cloned(),
            head_seq: v.transcript.head_seq(),
            items: v.transcript.since(since),
            stderr_tail: v.stderr_tail.clone(),
        })
    }

    /// Stop the session and its agent process.
    pub fn close(&self, thread: &ThreadId) -> Result<(), AcpError> {
        let commands = self.commands(thread)?;
        // Closed from this moment, not when the actor gets around to it.
        if let Some(h) = self.sessions.lock().get(thread) {
            h.closed.store(true, Ordering::SeqCst);
        }
        commands.send(Command::Close).map_err(|_| AcpError::NotOpen)
    }
}
