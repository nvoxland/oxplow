//! Every open chat session, by agent session: a Codex thread's. Holds only each session's command sender and shared view; the
//! actor (`session.rs`) owns the connection. Events for every session go
//! out on one broadcast channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oxplow_domain::AgentSessionId;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::codex::{CodexConnection, CodexOpen};
use super::conn::{Connection, Incoming};
use super::host::ChatHost;
use super::model::ContextUsage;
use super::session::{Actor, ChatError, ChatEvent, ChatStatus, Command, SessionSpec, SessionView};
use super::transcript::TranscriptItem;

/// A session as the UI reads it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatSnapshot {
    pub agent: String,
    /// The session's generation: a new one (a Restart) replaces the
    /// client's transcript instead of merging into it.
    #[specta(type = oxplow_domain::JsNumber)]
    pub generation: u64,
    pub status: ChatStatus,
    pub usage: Option<ContextUsage>,
    /// The highest `seq` in the transcript; ask `since` this next time.
    #[specta(type = oxplow_domain::JsNumber)]
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
    /// records nothing on its way out (the new one owns the session).
    current: Arc<AtomicBool>,
}

impl Handle {
    fn alive(&self) -> bool {
        !self.closed.load(Ordering::SeqCst) && !self.commands.is_closed()
    }
}

/// An agent session's slot, claimed before anything is spawned (see
/// [`ChatManager::reserve`]).
struct Reservation {
    session: AgentSessionId,
    generation: u64,
    commands: mpsc::UnboundedReceiver<Command>,
    view: Arc<Mutex<SessionView>>,
    current: Arc<AtomicBool>,
}

pub struct ChatManager {
    sessions: Mutex<HashMap<AgentSessionId, Handle>>,
    events: broadcast::Sender<ChatEvent>,
    /// Hands out session generations; one per open.
    next_generation: std::sync::atomic::AtomicU64,
}

impl Default for ChatManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatManager {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            next_generation: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChatEvent> {
        self.events.subscribe()
    }

    /// Publish an event as a session would (transport tests).
    pub fn emit_event_for_tests(&self, event: ChatEvent) {
        let _ = self.events.send(event);
    }

    /// Is agent session `session`'s agent running?
    pub fn is_open(&self, session: &AgentSessionId) -> bool {
        self.sessions.lock().get(session).is_some_and(Handle::alive)
    }

    /// Claim the session's slot for a new run, under one lock, before
    /// anything is spawned: two concurrent opens can't both start an
    /// agent. `None` when a session is already live. A closed session
    /// still winding down is replaced and marked not current.
    fn reserve(&self, spec: &SessionSpec) -> Option<Reservation> {
        let mut sessions = self.sessions.lock();
        if let Some(old) = sessions.get(&spec.session_id) {
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
            spec.session_id,
            Handle {
                commands: tx,
                view: view.clone(),
                generation,
                closed: Arc::new(AtomicBool::new(false)),
                current: current.clone(),
            },
        );
        Some(Reservation {
            session: spec.session_id,
            generation,
            commands: rx,
            view,
            current,
        })
    }

    /// Give up a reservation whose start failed (only if it's still ours).
    fn release(&self, session: &AgentSessionId, generation: u64) {
        let mut sessions = self.sessions.lock();
        if sessions
            .get(session)
            .is_some_and(|h| h.generation == generation)
        {
            sessions.remove(session);
        }
    }

    /// Open the session's thread on Codex's app-server daemon and observe
    /// it; its id, what Codex's TUI attaches to. A session already running
    /// is kept, and its thread's id answered.
    pub async fn open_codex(
        &self,
        host: Arc<dyn ChatHost>,
        spec: SessionSpec,
        open: CodexOpen,
    ) -> Result<String, ChatError> {
        let session = spec.session_id;
        if let Some(reservation) = self.reserve(&spec) {
            let (conn, incoming) = match CodexConnection::connect(open).await {
                Ok(c) => c,
                Err(e) => {
                    self.release(&reservation.session, reservation.generation);
                    return Err(ChatError::Agent(e));
                }
            };
            self.start(host, spec, reservation, conn, incoming).await?;
        }
        self.sessions
            .lock()
            .get(&session)
            .and_then(|h| h.view.lock().session_id.clone())
            .ok_or(ChatError::NotOpen)
    }

    fn generation(&self) -> u64 {
        self.next_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Run a session on a connection already made (tests: an in-process
    /// agent). A session already running is kept (`Ok`).
    #[cfg(test)]
    pub(super) async fn open_connection(
        &self,
        host: Arc<dyn ChatHost>,
        spec: SessionSpec,
        conn: Arc<dyn Connection>,
        incoming: mpsc::UnboundedReceiver<Incoming>,
    ) -> Result<(), ChatError> {
        let Some(reservation) = self.reserve(&spec) else {
            return Ok(());
        };
        self.start(host, spec, reservation, conn, incoming).await
    }

    /// Run the session's actor on an open connection.
    async fn start(
        &self,
        host: Arc<dyn ChatHost>,
        spec: SessionSpec,
        reservation: Reservation,
        conn: Arc<dyn Connection>,
        incoming: mpsc::UnboundedReceiver<Incoming>,
    ) -> Result<(), ChatError> {
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
            actor.run(conn, incoming, cmd_rx, ready_tx).await;
            teardown
                .run(Some("the agent connection ended".to_string()))
                .await;
        });
        Self::ready(ready_rx, &view).await
    }

    /// Whether the session started: the actor's word, else the agent's
    /// last stderr lines.
    async fn ready(
        ready: oneshot::Receiver<Result<(), ChatError>>,
        view: &Mutex<SessionView>,
    ) -> Result<(), ChatError> {
        match ready.await {
            Ok(r) => r,
            Err(_) => {
                let tail = view.lock().stderr_tail.join("\n");
                Err(ChatError::Agent(if tail.is_empty() {
                    "the agent exited during startup".into()
                } else {
                    format!("the agent exited during startup:\n{tail}")
                }))
            }
        }
    }

    fn commands(
        &self,
        session: &AgentSessionId,
    ) -> Result<mpsc::UnboundedSender<Command>, ChatError> {
        self.sessions
            .lock()
            .get(session)
            .filter(|h| h.alive())
            .map(|h| h.commands.clone())
            .ok_or(ChatError::NotOpen)
    }

    /// Send what a person typed. The only caller is the UI-only RPC
    /// command behind the prompt box (the source guard pins this).
    pub async fn submit_human_prompt(
        &self,
        session: &AgentSessionId,
        text: String,
    ) -> Result<(), ChatError> {
        let (reply, rx) = oneshot::channel();
        self.commands(session)?
            .send(Command::Prompt { text, reply })
            .map_err(|_| ChatError::NotOpen)?;
        rx.await.map_err(|_| ChatError::NotOpen)?
    }

    pub fn cancel(&self, session: &AgentSessionId) -> Result<(), ChatError> {
        self.commands(session)?
            .send(Command::Cancel)
            .map_err(|_| ChatError::NotOpen)
    }

    /// Name the session's agent conversation (what its own UI lists it
    /// as), when its agent can; the person's rename asks for it.
    pub async fn rename_native(
        &self,
        session: &AgentSessionId,
        name: String,
    ) -> Result<(), ChatError> {
        let (reply, rx) = oneshot::channel();
        self.commands(session)?
            .send(Command::Rename { name, reply })
            .map_err(|_| ChatError::NotOpen)?;
        rx.await.map_err(|_| ChatError::NotOpen)?
    }

    /// Answer a permission card; `option_id: None` cancels it.
    pub async fn respond_permission(
        &self,
        session: &AgentSessionId,
        request_id: String,
        option_id: Option<String>,
    ) -> Result<(), ChatError> {
        let (reply, rx) = oneshot::channel();
        self.commands(session)?
            .send(Command::Respond {
                request_id,
                option_id,
                reply,
            })
            .map_err(|_| ChatError::NotOpen)?;
        rx.await.map_err(|_| ChatError::NotOpen)?
    }

    /// The session's state and the items changed after `since`. Also works
    /// for a session that has stopped (its transcript stays readable).
    pub fn transcript(&self, session: &AgentSessionId, since: u64) -> Option<ChatSnapshot> {
        let sessions = self.sessions.lock();
        let v = sessions.get(session)?.view.lock();
        Some(ChatSnapshot {
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
    pub fn close(&self, session: &AgentSessionId) -> Result<(), ChatError> {
        let commands = self.commands(session)?;
        // Closed from this moment, not when the actor gets around to it.
        if let Some(h) = self.sessions.lock().get(session) {
            h.closed.store(true, Ordering::SeqCst);
        }
        commands
            .send(Command::Close)
            .map_err(|_| ChatError::NotOpen)
    }
}
