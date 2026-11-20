//! One observed session: an actor task that owns the agent's connection (a
//! Codex thread's: [`Connection`]) and records what the agent does the way
//! a hooked terminal agent's turns are recorded. It sends the agent nothing
//! but context between turns and a rename the person asked for; the person
//! prompts and answers in the agent's own UI. The manager (`manager.rs`)
//! holds only its command sender and the shared [`SessionView`].
//!
//! The rules it keeps (see `.context/agent-model.md` → "Codex: its TUI on a
//! thread oxplow observes"):
//! - a replay (a resumed thread's turns) records nothing;
//! - a prompt the person typed in the agent's UI opens a turn, recorded as
//!   theirs; a turn that starts with no prompt is nobody's;
//! - context reaches the agent only while no turn runs.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_domain::ThreadId;
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use super::conn::{Connection, Incoming, TurnEnd};
use super::host::ChatHost;
use super::mapping;
use super::model::{ChatUpdate, ToolCall, ToolStatus};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    #[error("no agent is running for this session")]
    NotOpen,
    #[error("{0}")]
    Agent(String),
}

/// The session observed. How its agent opens is its connection's.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// The agent session it observes.
    pub session_id: oxplow_domain::AgentSessionId,
    pub thread_id: ThreadId,
    /// Its worktree.
    pub cwd: PathBuf,
}

/// What the manager reads without a round trip to the actor.
#[derive(Debug)]
pub struct SessionView {
    /// This session's generation: one per open, so a replaced run is told
    /// apart from its successor.
    pub generation: u64,
    /// The agent's own id for the conversation, once open.
    pub session_id: Option<String>,
    /// The session ended (its teardown ran).
    pub stopped: bool,
}

impl SessionView {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            session_id: None,
            stopped: false,
        }
    }
}

pub enum Command {
    /// Name the agent's conversation; the person asked.
    Rename {
        name: String,
        reply: oneshot::Sender<Result<(), ChatError>>,
    },
    Close,
}

pub(super) struct Actor {
    spec: SessionSpec,
    /// False once a newer session replaced this one: its teardown then
    /// records nothing (the new session owns the thread's status).
    current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    host: Arc<dyn ChatHost>,
    view: Arc<Mutex<SessionView>>,
    session_id: String,
    /// A turn is in flight (the person's prompt opened it).
    turn: bool,
    /// The agent's own id for the turn in flight, once it said.
    harness_turn: Option<String>,
    /// Each tool call this run has seen, merged with its updates.
    tools: HashMap<String, ToolCall>,
    /// Tool calls whose start is on the record.
    started: HashSet<String>,
    /// Tool calls already recorded as finished.
    recorded: HashSet<String>,
    /// The agent's text since its last tool call or thought: what the
    /// turn answers, once it ends.
    answer: String,
    /// The last update was the agent's text, so the next chunk continues it.
    answering: bool,
    replaying: bool,
    /// A name the agent gave the conversation while it replayed, recorded
    /// once the session is.
    replayed_name: Option<String>,
    /// A turn ended or the context was summarized since the context was
    /// last offered to the agent's channel (`inject_context`).
    inject_due: bool,
}

impl Actor {
    pub(super) fn new(
        spec: SessionSpec,
        host: Arc<dyn ChatHost>,
        view: Arc<Mutex<SessionView>>,
        current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            spec,
            current,
            host,
            view,
            session_id: String::new(),
            turn: false,
            harness_turn: None,
            tools: HashMap::new(),
            started: HashSet::new(),
            recorded: HashSet::new(),
            answer: String::new(),
            answering: false,
            replaying: false,
            replayed_name: None,
            inject_due: false,
        }
    }

    fn thread(&self) -> &ThreadId {
        &self.spec.thread_id
    }

    /// Run the session until it is closed or the agent goes away.
    /// `ready` learns whether it started.
    pub(super) async fn run(
        mut self,
        conn: Arc<dyn Connection>,
        mut incoming: mpsc::UnboundedReceiver<Incoming>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        ready: oneshot::Sender<Result<(), ChatError>>,
    ) {
        if let Err(e) = self.start(conn.as_ref(), &mut incoming).await {
            self.view.lock().stopped = true;
            let _ = ready.send(Err(ChatError::Agent(e)));
            return;
        }
        let _ = ready.send(Ok(()));
        self.inject_context(conn.as_ref()).await;

        let closed = conn.closed();
        tokio::pin!(closed);
        // What the context says may change while no turn runs: build it
        // again on the signal (the baselines drop a build that changes
        // nothing).
        let mut changes = self.host.context_changes();
        loop {
            tokio::select! {
                changed = async {
                    match changes.as_mut() {
                        Some(rx) => rx.changed().await.is_ok(),
                        None => std::future::pending().await,
                    }
                } => {
                    if changed {
                        self.inject_context(conn.as_ref()).await;
                    } else {
                        changes = None;
                    }
                }
                msg = incoming.recv() => match msg {
                    Some(msg) => {
                        self.on_incoming(msg).await;
                        if self.inject_due {
                            self.inject_context(conn.as_ref()).await;
                        }
                    }
                    None => break,
                },
                cmd = commands.recv() => match cmd {
                    None | Some(Command::Close) => break,
                    Some(Command::Rename { name, reply }) => {
                        let _ = reply.send(conn.rename(&name).await.map_err(ChatError::Agent));
                    }
                },
                _ = &mut closed => {
                    // Record what arrived before the EOF first.
                    while let Ok(msg) = incoming.try_recv() {
                        self.on_incoming(msg).await;
                    }
                    break;
                }
            }
        }
        self.turn = false;
        self.teardown_handle().run().await;
    }

    async fn start(
        &mut self,
        conn: &dyn Connection,
        incoming: &mut mpsc::UnboundedReceiver<Incoming>,
    ) -> Result<(), String> {
        self.replaying = true;
        let opened = conn.open(&self.spec.cwd).await;
        while let Ok(msg) = incoming.try_recv() {
            self.on_incoming(msg).await;
        }
        self.replaying = false;
        let session_id = opened?;
        self.session_id = session_id.clone();
        self.view.lock().session_id = Some(session_id.clone());
        self.host.session_started(self.thread(), &session_id).await;
        if let Some(title) = self.replayed_name.take() {
            self.host.named(self.thread(), &session_id, &title).await;
        }
        Ok(())
    }

    /// Context for the agent, through its history: what a prompt would
    /// have carried (the session context when it changed, the context
    /// basket, nudges). Only while no turn runs: an item injected into one
    /// lengthens it. A turn ending or a compaction brings it due again, so
    /// a change reaches the agent before the person's next prompt.
    async fn inject_context(&mut self, conn: &dyn Connection) {
        self.inject_due = false;
        if self.turn || !conn.can_inject() {
            return;
        }
        let Some(context) = self
            .host
            .prompt_context(self.thread(), &self.session_id)
            .await
        else {
            return;
        };
        match conn.inject(&context.text).await {
            Ok(()) => self.host.context_delivered(context).await,
            // Nothing was taken: the next chance builds it all again.
            Err(e) => tracing::warn!(error = %e, "context not delivered to the agent"),
        }
    }

    /// A prompt the person typed in the agent's own UI: it opens a turn.
    async fn person_prompt(&mut self, text: String) {
        self.host
            .turn_started(
                self.thread(),
                &self.session_id,
                &text,
                self.harness_turn.as_deref(),
            )
            .await;
        self.turn = true;
        self.answer.clear();
        self.answering = false;
    }

    async fn on_incoming(&mut self, msg: Incoming) {
        if !self.replaying {
            self.host.activity(self.thread());
        }
        match msg {
            Incoming::Update(update) => self.on_update(*update).await,
            Incoming::Named(title) if self.replaying => self.replayed_name = Some(title),
            Incoming::Named(title) => {
                self.host
                    .named(self.thread(), &self.session_id, &title)
                    .await
            }
            // The agent's own UI is where it is answered; the status only
            // says it waits.
            Incoming::Waiting(_) if self.replaying => {}
            Incoming::Waiting(question) => self.host.awaiting_user(self.thread(), question).await,
            // A replayed history's children were recorded as they ran.
            Incoming::Subagent { .. } | Incoming::ChildCall { .. } if self.replaying => {}
            Incoming::TurnStarted(id) if self.replaying => self.harness_turn = Some(id),
            Incoming::TurnStarted(id) => {
                // A turn the person's prompt already opened takes the id
                // now; one whose prompt comes after it is opened with it.
                if self.turn {
                    self.host
                        .turn_identified(self.thread(), &self.session_id, &id)
                        .await;
                }
                self.harness_turn = Some(id);
            }
            Incoming::Subagent {
                change,
                id,
                kind,
                spawned_in,
            } => {
                self.host
                    .subagent(
                        self.thread(),
                        &self.session_id,
                        &change,
                        &id,
                        kind.as_deref(),
                        spawned_in.as_deref(),
                    )
                    .await
            }
            Incoming::ChildCall { child, call } => {
                if let Some(mut use_) = mapping::tool_use(&call) {
                    use_.subagent = Some(child);
                    self.host
                        .tool_finished(
                            self.thread(),
                            &self.session_id,
                            &use_,
                            &mapping::content(&call),
                        )
                        .await;
                }
            }
            Incoming::Compacted if self.replaying => {}
            Incoming::Compacted => {
                self.host.compacted(self.thread(), &self.session_id).await;
                self.inject_due = true;
            }
            Incoming::TurnEnded(_) if self.replaying => {}
            Incoming::TurnEnded(result) => self.on_turn_end(result).await,
        }
    }

    async fn on_update(&mut self, update: ChatUpdate) {
        match update {
            // One with no turn in flight is a prompt the person typed in the
            // agent's own UI: theirs, and the turn it starts is recorded as
            // any other.
            ChatUpdate::UserChunk(text) => {
                if !self.replaying && !self.turn {
                    self.person_prompt(text).await;
                }
            }
            ChatUpdate::AgentChunk(text) => {
                if !self.answering {
                    self.answer.clear();
                }
                self.answer.push_str(&text);
                self.answering = true;
            }
            ChatUpdate::ThoughtChunk => self.answering = false,
            ChatUpdate::ToolCall(call) => {
                // A new call ends the text before it; an update of a known
                // one doesn't.
                if !self.tools.contains_key(&call.id) {
                    self.answering = false;
                }
                let id = call.id.clone();
                self.tools.insert(id.clone(), call);
                self.tool_changed(&id).await;
            }
            ChatUpdate::ToolCallUpdate(patch) => {
                let id = patch.id.clone();
                match self.tools.get_mut(&id) {
                    Some(call) => call.apply(&patch),
                    None => {
                        self.answering = false;
                        self.tools.insert(id.clone(), ToolCall::from_patch(&patch));
                    }
                }
                self.tool_changed(&id).await;
            }
            ChatUpdate::Ignored => {}
        }
    }

    /// Record a call's start, then its finish; a write the agent made is
    /// checked against the policy as it lands (the agent's own UI approved
    /// it), so a denial is on the record.
    async fn tool_changed(&mut self, id: &str) {
        if self.replaying {
            return;
        }
        let Some(tool) = self.tools.get(id).cloned() else {
            return;
        };
        if !matches!(tool.status, ToolStatus::Completed | ToolStatus::Failed) {
            if let (true, Some(call)) = (
                self.started.insert(id.to_string()),
                mapping::tool_use(&tool),
            ) {
                self.host
                    .tool_started(self.thread(), &self.session_id, &call)
                    .await;
            }
            return;
        }
        if !self.recorded.insert(id.to_string()) {
            return;
        }
        let Some(call) = mapping::tool_use(&tool) else {
            return;
        };
        let content = mapping::content(&tool);
        if mapping::is_write(tool.kind) && tool.status == ToolStatus::Completed {
            if let PolicyDecision::Deny { reason, .. } = self
                .host
                .check_tool(self.thread(), &self.session_id, &call, &content)
                .await
            {
                tracing::info!(call = %call.name, %reason, "an observed write the policy denies");
            }
        }
        self.host
            .tool_finished(self.thread(), &self.session_id, &call, &content)
            .await;
    }

    async fn on_turn_end(&mut self, result: Result<TurnEnd, String>) {
        self.turn = false;
        self.harness_turn = None;
        self.inject_due = true;
        // How the agent says it ended: a turn it cancelled is interrupted, a
        // turn that failed is errored with the agent's words.
        let (tokens, run) = match &result {
            Ok(end) => {
                let mut run = end.run.clone();
                if matches!(end.stop_reason.as_str(), "cancelled" | "interrupted") {
                    run.get_or_insert_with(Default::default).outcome =
                        Some(oxplow_domain::hook::TurnOutcome::Interrupted);
                }
                (end.usage.clone(), run)
            }
            Err(e) => (
                None,
                Some(oxplow_domain::hook::TurnRun {
                    outcome: Some(oxplow_domain::hook::TurnOutcome::Errored),
                    error: Some(e.clone()),
                    ..Default::default()
                }),
            ),
        };
        let answer = std::mem::take(&mut self.answer);
        self.answering = false;
        let answer = (!answer.trim().is_empty()).then_some(answer);
        self.host
            .turn_ended(
                self.thread(),
                &self.session_id,
                answer.as_deref(),
                tokens.as_ref(),
                run.as_ref(),
            )
            .await;
    }

    pub(super) fn teardown_handle(&self) -> Teardown {
        Teardown {
            thread: self.spec.thread_id,
            view: self.view.clone(),
            host: self.host.clone(),
            current: self.current.clone(),
        }
    }
}

/// Everything ending a session needs, apart from the actor: the actor runs
/// it on a clean exit, and the task driving the connection runs it when
/// the actor never got to. Idempotent: a session already stopped is left
/// alone.
pub(super) struct Teardown {
    thread: ThreadId,
    view: Arc<Mutex<SessionView>>,
    host: Arc<dyn ChatHost>,
    current: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Teardown {
    pub(super) async fn run(&self) {
        {
            let mut v = self.view.lock();
            if v.stopped {
                return;
            }
            v.stopped = true;
        }
        // A newer session owns the thread's status once this one is replaced.
        if self.current.load(std::sync::atomic::Ordering::SeqCst) {
            self.host.interrupted(&self.thread).await;
        }
    }
}
