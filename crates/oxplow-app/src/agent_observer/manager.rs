//! Every observed session, by agent session: a Codex thread's. Holds only
//! each session's command sender and shared view; the actor (`session.rs`)
//! owns the connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use oxplow_domain::AgentSessionId;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use super::codex::{CodexConnection, CodexOpen};
use super::conn::{Connection, Incoming};
use super::host::ObserverHost;
use super::session::{Actor, ChatError, Command, SessionSpec, SessionView};

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
/// [`ObserverManager::reserve`]).
struct Reservation {
    session: AgentSessionId,
    generation: u64,
    commands: mpsc::UnboundedReceiver<Command>,
    view: Arc<Mutex<SessionView>>,
    current: Arc<AtomicBool>,
}

pub struct ObserverManager {
    sessions: Mutex<HashMap<AgentSessionId, Handle>>,
    /// Hands out session generations; one per open.
    next_generation: std::sync::atomic::AtomicU64,
}

impl Default for ObserverManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ObserverManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_generation: std::sync::atomic::AtomicU64::new(1),
        }
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
        let view = Arc::new(Mutex::new(SessionView::new(generation)));
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
        host: Arc<dyn ObserverHost>,
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
        host: Arc<dyn ObserverHost>,
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
        host: Arc<dyn ObserverHost>,
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
        let actor = Actor::new(spec, host, view, current);
        let teardown = actor.teardown_handle();
        tokio::spawn(async move {
            actor.run(conn, incoming, cmd_rx, ready_tx).await;
            teardown.run().await;
        });
        ready_rx
            .await
            .unwrap_or_else(|_| Err(ChatError::Agent("the agent connection ended".into())))
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
