//! Every open ACP session, by thread. Holds only each session's command
//! sender and shared view; the actor (`session.rs`) owns the connection.
//! Events for every session go out on one broadcast channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use oxplow_domain::ThreadId;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::host::AcpHost;
use super::model::ContextUsage;
use super::session::{
    AcpError, AcpEvent, AcpEventBody, AcpStatus, Actor, Command, SessionSpec, SessionView,
};
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
    pub status: AcpStatus,
    pub directive: Option<String>,
    pub usage: Option<ContextUsage>,
    /// The highest `seq` in the transcript; ask `since` this next time.
    pub head_seq: u64,
    pub items: Vec<TranscriptItem>,
    pub stderr_tail: Vec<String>,
}

struct Handle {
    commands: mpsc::UnboundedSender<Command>,
    view: Arc<Mutex<SessionView>>,
}

pub struct AcpManager {
    sessions: Mutex<HashMap<ThreadId, Handle>>,
    events: broadcast::Sender<AcpEvent>,
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
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AcpEvent> {
        self.events.subscribe()
    }

    /// Is a session running for `thread`?
    pub fn is_open(&self, thread: &ThreadId) -> bool {
        self.sessions
            .lock()
            .get(thread)
            .is_some_and(|h| !h.commands.is_closed())
    }

    /// Start the agent process and its session. A session already running
    /// for the thread is kept (`Ok`).
    pub async fn open(
        &self,
        host: Arc<dyn AcpHost>,
        spec: SessionSpec,
        launch: Launch,
    ) -> Result<(), AcpError> {
        if self.is_open(&spec.thread_id) {
            return Ok(());
        }
        let mut cmd = tokio::process::Command::new(&launch.program);
        cmd.args(&launch.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(path) = crate::agent_path::augmented_path() {
            cmd.env("PATH", path);
        }
        for (k, v) in &launch.env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AcpError::Agent(format!("starting {}: {e}", launch.program.display())))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(AcpError::Agent("the agent's stdio is unavailable".into()));
        };
        let view = Arc::new(Mutex::new(SessionView::new(&spec.agent)));
        if let Some(stderr) = child.stderr.take() {
            let view = view.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    view.lock().push_stderr(line);
                }
            });
        }
        self.start(host, spec, view, stdin, stdout, Some(child))
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
        if self.is_open(&spec.thread_id) {
            return Ok(());
        }
        let view = Arc::new(Mutex::new(SessionView::new(&spec.agent)));
        self.start(host, spec, view, write, read, None).await
    }

    async fn start<W, R>(
        &self,
        host: Arc<dyn AcpHost>,
        spec: SessionSpec,
        view: Arc<Mutex<SessionView>>,
        write: W,
        read: R,
        child: Option<tokio::process::Child>,
    ) -> Result<(), AcpError>
    where
        W: AsyncWrite + Send + 'static,
        R: AsyncRead + Send + 'static,
    {
        let thread = spec.thread_id;
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let actor = Actor::new(spec, host, view.clone(), self.events.clone());
        self.sessions.lock().insert(
            thread,
            Handle {
                commands: cmd_tx,
                view: view.clone(),
            },
        );
        tokio::spawn(async move {
            let r = wire::run(write, read, move |conn, incoming| {
                actor.run(conn, incoming, cmd_rx, ready_tx)
            })
            .await;
            if let Err(e) = r {
                tracing::warn!(error = %e, "acp: connection ended with an error");
            }
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
            .filter(|h| !h.commands.is_closed())
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
            status: v.status,
            directive: v.directive.clone(),
            usage: v.transcript.usage().cloned(),
            head_seq: v.transcript.head_seq(),
            items: v.transcript.since(since),
            stderr_tail: v.stderr_tail.clone(),
        })
    }

    /// The person dismissed the directive banner.
    pub fn dismiss_directive(&self, thread: &ThreadId) -> Result<(), AcpError> {
        let sessions = self.sessions.lock();
        let h = sessions.get(thread).ok_or(AcpError::NotOpen)?;
        h.view.lock().directive = None;
        let _ = self.events.send(AcpEvent {
            thread_id: thread.to_string(),
            body: AcpEventBody::Directive { text: None },
        });
        Ok(())
    }

    /// Stop the session and its agent process.
    pub fn close(&self, thread: &ThreadId) -> Result<(), AcpError> {
        self.commands(thread)?
            .send(Command::Close)
            .map_err(|_| AcpError::NotOpen)
    }
}
