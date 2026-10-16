//! One chat session: an actor task that owns the agent's connection (an
//! ACP agent's, a Codex thread's: [`Connection`]) and turns its traffic
//! into transcript items, policy decisions and recording. The manager
//! (`manager.rs`) holds only its command sender and the shared
//! [`SessionView`].
//!
//! The rules it keeps (see `.context/agent-model.md` → "ACP agents"):
//! - it sends a prompt only for [`Command::Prompt`], which comes only from
//!   a person's submit; a second prompt while a turn runs is an error,
//!   never queued;
//! - a tool call the policy denies is rejected without asking; anything
//!   else waits on the person, with no timeout and no "always allow" for
//!   writes;
//! - a replay (`session/load`, a resumed thread's turns) rebuilds the
//!   transcript and records nothing;
//! - a person's prompt typed elsewhere on the same conversation (Codex's
//!   own TUI on the thread) is recorded as theirs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::agent::tool::ToolUse;
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::host::ChatHost;

/// What a write oxplow makes for an ACP agent (`fs/write_text_file`) is
/// called, where a person reads it and in the record.
const FS_WRITE: &str = "fs/write_text_file";
use super::conn::{Connection, Incoming, PermissionReply, TurnEnd};
use super::mapping;
use super::model::{
    ChatUpdate, ContextUsage, PermissionAnswer, PermissionKind, PermissionOption, ToolCall,
    ToolStatus,
};
use super::transcript::{ItemBody, Transcript, TranscriptItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatStatus {
    Starting,
    Idle,
    Running,
    AwaitingPermission,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatEventBody {
    Item { item: Box<TranscriptItem> },
    Status { status: ChatStatus },
    Usage { usage: ContextUsage },
    Closed { reason: Option<String> },
}

/// A change to one agent session's ACP agent, pushed to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    /// The agent session (`ses3`) it is about.
    pub agent_session_id: String,
    /// Its thread, for a client that filters by thread.
    pub thread_id: String,
    /// Which run of the session: each open is a new generation whose
    /// ids and seqs start over, so a client resets rather than merging it
    /// with the last one's transcript.
    #[specta(type = oxplow_domain::JsNumber)]
    pub generation: u64,
    #[serde(flatten)]
    pub body: ChatEventBody,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    #[error("no agent is running for this session")]
    NotOpen,
    #[error("the agent is still working on the last prompt; wait for it or stop it")]
    TurnInFlight,
    #[error("no pending permission request {0}")]
    UnknownRequest(String),
    #[error("{0} is not one of the offered options")]
    UnknownOption(String),
    #[error("the prompt is empty")]
    EmptyPrompt,
    #[error("{0}")]
    Agent(String),
}

/// The session a chat runs. How its agent opens is its connection's.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// The agent session it runs.
    pub session_id: oxplow_domain::AgentSessionId,
    pub thread_id: ThreadId,
    /// What runs it, as a person names it: the `acpAgents` name, or the
    /// harness's.
    pub agent: String,
    /// Its worktree.
    pub cwd: PathBuf,
}

/// What the manager and the UI read without a round trip to the actor.
#[derive(Debug)]
pub struct SessionView {
    pub agent: String,
    /// This session's generation (see [`ChatEvent::generation`]).
    pub generation: u64,
    pub status: ChatStatus,
    pub transcript: Transcript,
    pub session_id: Option<String>,
    /// The agent's last stderr lines, for a failed start or a crash.
    pub stderr_tail: Vec<String>,
}

pub const STDERR_TAIL: usize = 64;

impl SessionView {
    pub fn new(agent: &str, generation: u64) -> Self {
        Self {
            agent: agent.to_string(),
            generation,
            status: ChatStatus::Starting,
            transcript: Transcript::default(),
            session_id: None,
            stderr_tail: Vec::new(),
        }
    }

    pub fn push_stderr(&mut self, line: String) {
        self.stderr_tail.push(line);
        if self.stderr_tail.len() > STDERR_TAIL {
            self.stderr_tail.remove(0);
        }
    }
}

pub enum Command {
    Prompt {
        text: String,
        reply: oneshot::Sender<Result<(), ChatError>>,
    },
    Cancel,
    /// The thread's access may have changed: tell the connection what it
    /// is now.
    SyncAccess,
    Respond {
        request_id: String,
        option_id: Option<String>,
        reply: oneshot::Sender<Result<(), ChatError>>,
    },
    Close,
}

struct Pending {
    reply: PermissionReply,
    item_id: u64,
    tool_call_id: String,
    options: Vec<PermissionOption>,
}

pub(super) struct Actor {
    spec: SessionSpec,
    generation: u64,
    /// False once a newer session replaced this one: its teardown then
    /// records nothing (the new session owns the thread's status).
    current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    host: Arc<dyn ChatHost>,
    view: Arc<Mutex<SessionView>>,
    events: broadcast::Sender<ChatEvent>,
    session_id: String,
    /// The prompt of the turn in flight.
    turn: Option<String>,
    pending: HashMap<String, Pending>,
    next_request: u64,
    /// Tool calls a person allowed (their writes are theirs to make).
    allowed: HashSet<String>,
    /// Tool calls the policy or a person rejected (or cancelled): if one
    /// completes anyway, the agent ignored the answer.
    rejected: HashSet<String>,
    /// Tool calls already recorded as finished.
    recorded: HashSet<String>,
    /// Paths written through `fs/write_text_file` this turn.
    fs_written: HashSet<String>,
    /// The system prompt, when it rides the first prompt.
    pending_system_prompt: Option<String>,
    replaying: bool,
    /// A name the agent gave the conversation while it replayed, recorded
    /// once the session is.
    replayed_name: Option<String>,
    /// What the connection was last told the thread may write.
    writable: bool,
}

impl Actor {
    pub(super) fn new(
        spec: SessionSpec,
        host: Arc<dyn ChatHost>,
        view: Arc<Mutex<SessionView>>,
        events: broadcast::Sender<ChatEvent>,
        current: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let generation = view.lock().generation;
        Self {
            spec,
            generation,
            current,
            host,
            view,
            events,
            session_id: String::new(),
            turn: None,
            pending: HashMap::new(),
            next_request: 0,
            allowed: HashSet::new(),
            rejected: HashSet::new(),
            recorded: HashSet::new(),
            fs_written: HashSet::new(),
            pending_system_prompt: None,
            replaying: false,
            replayed_name: None,
            writable: false,
        }
    }

    fn emit(&self, body: ChatEventBody) {
        let _ = self.events.send(ChatEvent {
            agent_session_id: self.spec.session_id.to_string(),
            thread_id: self.spec.thread_id.to_string(),
            generation: self.generation,
            body,
        });
    }

    fn push(&self, body: ItemBody) -> TranscriptItem {
        let item = self.view.lock().transcript.push(body);
        self.emit(ChatEventBody::Item {
            item: Box::new(item.clone()),
        });
        item
    }

    fn set_status(&self, status: ChatStatus) {
        let changed = {
            let mut v = self.view.lock();
            let changed = v.status != status;
            v.status = status;
            changed
        };
        if changed {
            self.emit(ChatEventBody::Status { status });
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
            self.push(ItemBody::Error { message: e.clone() });
            self.set_status(ChatStatus::Stopped);
            self.emit(ChatEventBody::Closed {
                reason: Some(e.clone()),
            });
            let _ = ready.send(Err(ChatError::Agent(e)));
            return;
        }
        let _ = ready.send(Ok(()));

        let closed = conn.closed();
        tokio::pin!(closed);
        let reason = loop {
            tokio::select! {
                msg = incoming.recv() => match msg {
                    Some(msg) => self.on_incoming(msg).await,
                    None => break Some("the agent connection closed".to_string()),
                },
                cmd = commands.recv() => match cmd {
                    None | Some(Command::Close) => break None,
                    Some(cmd) => self.on_command(conn.as_ref(), cmd).await,
                },
                _ = &mut closed => {
                    // Deliver what arrived before the EOF first.
                    while let Ok(msg) = incoming.try_recv() {
                        self.on_incoming(msg).await;
                    }
                    break Some("the agent exited".to_string());
                }
            }
        };
        self.shut_down(reason).await;
    }

    async fn start(
        &mut self,
        conn: &dyn Connection,
        incoming: &mut mpsc::UnboundedReceiver<Incoming>,
    ) -> Result<(), String> {
        let writable = self.host.writable(self.thread()).await;
        self.writable = writable;
        self.replaying = true;
        let opened = conn.open(&self.spec.cwd, writable).await;
        while let Ok(msg) = incoming.try_recv() {
            self.on_incoming(msg).await;
        }
        self.replaying = false;
        let opened = opened?;
        self.pending_system_prompt = opened.preamble;
        self.session_id = opened.session_id.clone();
        self.view.lock().session_id = Some(opened.session_id.clone());
        self.host
            .session_started(self.thread(), &opened.session_id)
            .await;
        if let Some(title) = self.replayed_name.take() {
            self.host
                .named(self.thread(), &opened.session_id, &title)
                .await;
        }
        self.set_status(ChatStatus::Idle);
        Ok(())
    }

    async fn on_command(&mut self, conn: &dyn Connection, cmd: Command) {
        match cmd {
            Command::Prompt { text, reply } => {
                let r = self.prompt(conn, text).await;
                let _ = reply.send(r);
            }
            Command::Cancel => {
                if self.turn.is_some() {
                    conn.cancel();
                }
                // The protocol wants every open permission request
                // answered `cancelled` once the client cancels.
                if !self.pending.is_empty() {
                    self.cancel_pending();
                    self.host.awaiting_user(self.thread(), None).await;
                }
                // No card left waiting: the view says what's happening.
                self.set_status(if self.turn.is_some() {
                    ChatStatus::Running
                } else {
                    ChatStatus::Idle
                });
            }
            Command::Respond {
                request_id,
                option_id,
                reply,
            } => {
                let _ = reply.send(self.respond(&request_id, option_id).await);
            }
            // Only a change: a sandbox change restarts the thread's MCP
            // servers.
            Command::SyncAccess => {
                let writable = self.host.writable(self.thread()).await;
                if writable != self.writable {
                    self.writable = writable;
                    conn.set_writable(writable);
                }
            }
            Command::Close => {}
        }
    }

    /// The person's prompt: the one place a prompt is built and sent.
    async fn prompt(&mut self, conn: &dyn Connection, text: String) -> Result<(), ChatError> {
        if self.turn.is_some() {
            return Err(ChatError::TurnInFlight);
        }
        if text.trim().is_empty() {
            return Err(ChatError::EmptyPrompt);
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(sp) = self.pending_system_prompt.take() {
            parts.push(sp);
        }
        if let Some(c) = self
            .host
            .prompt_context(self.thread(), &self.session_id)
            .await
        {
            parts.push(c);
        }
        let context = (!parts.is_empty()).then(|| parts.join("\n\n"));
        let hp = super::human_prompt::compose(&text, context.as_deref());

        self.push(ItemBody::User {
            text: text.clone(),
            context,
        });
        self.host
            .turn_started(self.thread(), &self.session_id, &text)
            .await;
        self.fs_written.clear();
        self.turn = Some(text);
        let writable = self.host.writable(self.thread()).await;
        if let Err(e) = conn.prompt(hp, writable) {
            self.turn = None;
            self.push(ItemBody::Error { message: e.clone() });
            return Err(ChatError::Agent(e));
        }
        self.set_status(ChatStatus::Running);
        Ok(())
    }

    /// A person's prompt sent from another client of the conversation.
    async fn external_prompt(&mut self, text: String) {
        self.push(ItemBody::User {
            text: text.clone(),
            context: None,
        });
        self.host
            .turn_started(self.thread(), &self.session_id, &text)
            .await;
        self.fs_written.clear();
        self.turn = Some(text);
        self.set_status(ChatStatus::Running);
    }

    async fn respond(
        &mut self,
        request_id: &str,
        option_id: Option<String>,
    ) -> Result<(), ChatError> {
        let Some(p) = self.pending.remove(request_id) else {
            return Err(ChatError::UnknownRequest(request_id.to_string()));
        };
        let answer = match option_id {
            Some(id) if p.options.iter().any(|o| o.id == id) => {
                PermissionAnswer::Selected { option_id: id }
            }
            Some(id) => {
                self.pending.insert(request_id.to_string(), p);
                return Err(ChatError::UnknownOption(id));
            }
            None => PermissionAnswer::Cancelled,
        };
        p.reply.answer(&answer);
        let allows = matches!(&answer, PermissionAnswer::Selected { option_id }
            if p.options.iter().any(|o| &o.id == option_id
                && matches!(o.kind, PermissionKind::AllowOnce | PermissionKind::AllowAlways)));
        if allows {
            self.allowed.insert(p.tool_call_id.clone());
        } else {
            self.rejected.insert(p.tool_call_id.clone());
        }
        self.mark_answered(p.item_id, answer);
        if self.pending.is_empty() {
            self.host.awaiting_user(self.thread(), None).await;
            self.set_status(if self.turn.is_some() {
                ChatStatus::Running
            } else {
                ChatStatus::Idle
            });
        }
        Ok(())
    }

    fn mark_answered(&self, item_id: u64, a: PermissionAnswer) {
        let item = self.view.lock().transcript.update(item_id, |b| {
            if let ItemBody::Permission { answer, .. } = b {
                *answer = Some(a);
            }
        });
        if let Some(item) = item {
            self.emit(ChatEventBody::Item {
                item: Box::new(item),
            });
        }
    }

    fn cancel_pending(&mut self) {
        for (_, p) in self.pending.drain().collect::<Vec<_>>() {
            p.reply.answer(&PermissionAnswer::Cancelled);
            self.rejected.insert(p.tool_call_id.clone());
            self.mark_answered(p.item_id, PermissionAnswer::Cancelled);
        }
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
            Incoming::Permission { ask, reply } => {
                let ask = *ask;
                if self.replaying {
                    reply.answer(&PermissionAnswer::Cancelled);
                    return;
                }
                let tool_id = ask.tool.id.clone();
                self.apply(ChatUpdate::ToolCallUpdate(ask.tool.clone()));
                let tool = self
                    .view
                    .lock()
                    .transcript
                    .tool(&tool_id)
                    .cloned()
                    .unwrap_or_else(|| ToolCall::from_patch(&ask.tool));
                let call = mapping::tool_use(&tool).unwrap_or_else(|| ToolUse {
                    name: mapping::label(&tool),
                    ..ToolUse::default()
                });
                match self
                    .host
                    .check_tool(
                        self.thread(),
                        &self.session_id,
                        &call,
                        &mapping::content(&tool),
                    )
                    .await
                {
                    PolicyDecision::Deny { reason, .. } => {
                        let answer = match ask
                            .options
                            .iter()
                            .find(|o| o.kind == PermissionKind::RejectOnce)
                        {
                            Some(o) => PermissionAnswer::Selected {
                                option_id: o.id.clone(),
                            },
                            None => PermissionAnswer::Cancelled,
                        };
                        reply.answer(&answer);
                        self.rejected.insert(tool_id.clone());
                        self.push(ItemBody::PolicyDenied {
                            tool_call_id: tool_id,
                            label: call.name,
                            reason,
                        });
                    }
                    PolicyDecision::Allow => {
                        let write = call.kind == oxplow_domain::agent::tool::ToolKind::Edit;
                        let options: Vec<PermissionOption> = ask
                            .options
                            .into_iter()
                            .filter(|o| !(write && o.kind == PermissionKind::AllowAlways))
                            .collect();
                        self.next_request += 1;
                        let request_id = format!("perm-{}", self.next_request);
                        let title = if tool.title.is_empty() {
                            call.name.clone()
                        } else {
                            tool.title.clone()
                        };
                        let item = self.push(ItemBody::Permission {
                            request_id: request_id.clone(),
                            tool_call_id: tool_id.clone(),
                            title: title.clone(),
                            options: options.clone(),
                            answer: None,
                        });
                        self.pending.insert(
                            request_id,
                            Pending {
                                reply,
                                item_id: item.id,
                                tool_call_id: tool_id,
                                options,
                            },
                        );
                        self.set_status(ChatStatus::AwaitingPermission);
                        self.host
                            .awaiting_user(self.thread(), Some(format!("Permission: {title}")))
                            .await;
                    }
                }
            }
            Incoming::WriteFile {
                path,
                content,
                reply,
            } => {
                let path = match self.confined(&path) {
                    Ok(p) => p,
                    Err(reason) => {
                        reply.deny(&reason);
                        if !self.replaying {
                            self.push(ItemBody::PolicyDenied {
                                tool_call_id: String::new(),
                                label: FS_WRITE.into(),
                                reason,
                            });
                        }
                        return;
                    }
                };
                let p = path.to_string_lossy().into_owned();
                // oxplow writes it for the agent: an edit of that file.
                let call = ToolUse {
                    name: FS_WRITE.into(),
                    kind: oxplow_domain::agent::tool::ToolKind::Edit,
                    paths: vec![p.clone()],
                    ..ToolUse::default()
                };
                let payload = serde_json::json!({"tool_input": {"path": p}});
                let decision = if self.replaying {
                    PolicyDecision::Deny {
                        layer: oxplow_runtime::policy::DenyLayer::WriteGuard,
                        reason: "oxplow is replaying this session; nothing is written".into(),
                    }
                } else {
                    self.host
                        .check_tool(self.thread(), &self.session_id, &call, &payload)
                        .await
                };
                match decision {
                    PolicyDecision::Deny { reason, .. } => {
                        reply.deny(&reason);
                        if !self.replaying {
                            self.push(ItemBody::PolicyDenied {
                                tool_call_id: String::new(),
                                label: FS_WRITE.into(),
                                reason,
                            });
                        }
                    }
                    PolicyDecision::Allow => match write_file(&path, &content) {
                        Ok(()) => {
                            self.fs_written.insert(p);
                            reply.ok();
                        }
                        Err(e) => reply.deny(&format!("writing {p}: {e}")),
                    },
                }
            }
            Incoming::ReadFile {
                path,
                line,
                limit,
                reply,
            } => {
                let path = match self.confined(&path) {
                    Ok(p) => p,
                    Err(reason) => {
                        reply.err(&reason);
                        return;
                    }
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => reply.ok(slice_lines(&text, line, limit)),
                    Err(e) => reply.err(&format!("reading {}: {e}", path.display())),
                }
            }
            Incoming::PromptDone(result) => self.on_turn_end(result).await,
        }
    }

    /// `path` resolved for oxplow to read or write on the agent's
    /// behalf: normalized (`..`, symlinks) and inside this session's
    /// worktree. The daemon never touches files elsewhere for an agent
    /// (workspace isolation); the agent's own tools are its business.
    fn confined(&self, path: &Path) -> Result<PathBuf, String> {
        use oxplow_runtime::policy::normalize_path;
        let root = normalize_path(&self.spec.cwd, &self.spec.cwd);
        let resolved = normalize_path(path, &root);
        if resolved.starts_with(&root) {
            Ok(resolved)
        } else {
            Err(format!(
                "`{}` is outside this session's worktree (`{}`); oxplow only reads and writes \
                 files inside it",
                resolved.display(),
                root.display()
            ))
        }
    }

    fn apply(&self, update: ChatUpdate) -> Option<TranscriptItem> {
        let usage = matches!(update, ChatUpdate::Usage(_));
        let item = self.view.lock().transcript.apply(update);
        if let Some(item) = &item {
            self.emit(ChatEventBody::Item {
                item: Box::new(item.clone()),
            });
        }
        if usage {
            if let Some(u) = self.view.lock().transcript.usage().cloned() {
                self.emit(ChatEventBody::Usage { usage: u });
            }
        }
        item
    }

    async fn on_update(&mut self, update: ChatUpdate) {
        if let (false, ChatUpdate::UserChunk(text)) = (self.replaying, &update) {
            // The person's prompt from here is already an item (pushed
            // when it was sent); a live echo of it would be appended to
            // it. One with no turn in flight is a prompt the person typed
            // into another client of the same conversation: theirs, and
            // the turn it starts is recorded as any other.
            if self.turn.is_none() {
                self.external_prompt(text.clone()).await;
            }
            return;
        }
        let tool_id = match &update {
            ChatUpdate::ToolCall(t) => Some(t.id.clone()),
            ChatUpdate::ToolCallUpdate(p) => Some(p.id.clone()),
            _ => None,
        };
        self.apply(update);
        if self.replaying {
            return;
        }
        let Some(id) = tool_id else { return };
        let Some(tool) = self.view.lock().transcript.tool(&id).cloned() else {
            return;
        };
        if !matches!(tool.status, ToolStatus::Completed | ToolStatus::Failed)
            || !self.recorded.insert(id.clone())
        {
            return;
        }
        // A write that finished after being rejected ignored the answer.
        if mapping::is_write(tool.kind)
            && tool.status == ToolStatus::Completed
            && self.rejected.contains(&id)
        {
            self.push(ItemBody::Bypass {
                tool_call_id: id.clone(),
                label: mapping::label(&tool),
                reason: "it completed after being rejected; the agent ignored the answer".into(),
            });
        }
        // A write that finished without anyone allowing it and without going
        // through fs/write_text_file slipped past the gate: say so if the
        // policy would have refused it.
        else if mapping::is_write(tool.kind)
            && tool.status == ToolStatus::Completed
            && !self.allowed.contains(&id)
        {
            let paths = mapping::paths(&tool);
            let root = oxplow_runtime::policy::normalize_path(&self.spec.cwd, &self.spec.cwd);
            let unseen = paths.is_empty()
                || paths.iter().any(|p| {
                    let n = oxplow_runtime::policy::normalize_path(Path::new(p), &root);
                    !self.fs_written.contains(n.to_string_lossy().as_ref())
                });
            if let (true, Some(call)) = (unseen, mapping::tool_use(&tool)) {
                if let PolicyDecision::Deny { reason, .. } = self
                    .host
                    .check_tool(
                        self.thread(),
                        &self.session_id,
                        &call,
                        &mapping::content(&tool),
                    )
                    .await
                {
                    self.push(ItemBody::Bypass {
                        tool_call_id: id.clone(),
                        label: call.name,
                        reason,
                    });
                }
            }
        }
        if let Some(call) = mapping::tool_use(&tool) {
            self.host
                .tool_finished(
                    self.thread(),
                    &self.session_id,
                    &call,
                    &mapping::content(&tool),
                )
                .await;
        }
    }

    async fn on_turn_end(&mut self, result: Result<TurnEnd, String>) {
        // The turn is over (its prompt is on the `agent_turn` row).
        self.turn.take();
        // Clear the cards' "awaiting you" BEFORE the Stop is ingested, so
        // the Stop decides the status from the turn's final message.
        if !self.pending.is_empty() {
            self.cancel_pending();
            self.host.awaiting_user(self.thread(), None).await;
        }
        let tokens = match &result {
            Ok(end) => end.usage.clone(),
            Err(e) => {
                self.push(ItemBody::Error {
                    message: format!("the prompt failed: {e}"),
                });
                None
            }
        };
        let answer = self.view.lock().transcript.last_answer();
        self.host
            .turn_ended(
                self.thread(),
                &self.session_id,
                answer.as_deref(),
                tokens.as_ref(),
            )
            .await;
        self.set_status(ChatStatus::Idle);
    }

    async fn shut_down(&mut self, reason: Option<String>) {
        // Answers can't reach an agent that's gone; drop the replies.
        self.pending.clear();
        self.turn = None;
        self.teardown_handle().run(reason).await;
    }

    pub(super) fn teardown_handle(&self) -> Teardown {
        Teardown {
            session: self.spec.session_id,
            thread: self.spec.thread_id,
            generation: self.generation,
            view: self.view.clone(),
            host: self.host.clone(),
            events: self.events.clone(),
            current: self.current.clone(),
        }
    }
}

/// Everything ending a session needs, apart from the actor: the actor
/// runs it on a clean exit, and the task driving the connection runs it
/// when the actor never got to (a transport error drops the actor
/// mid-loop). Idempotent: a session already stopped is left alone.
pub(super) struct Teardown {
    session: oxplow_domain::AgentSessionId,
    thread: ThreadId,
    generation: u64,
    view: Arc<Mutex<SessionView>>,
    host: Arc<dyn ChatHost>,
    events: broadcast::Sender<ChatEvent>,
    current: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Teardown {
    fn emit(&self, body: ChatEventBody) {
        let _ = self.events.send(ChatEvent {
            agent_session_id: self.session.to_string(),
            thread_id: self.thread.to_string(),
            generation: self.generation,
            body,
        });
    }

    pub(super) async fn run(&self, reason: Option<String>) {
        let mut changed = Vec::new();
        {
            let mut v = self.view.lock();
            if v.status == ChatStatus::Stopped {
                return;
            }
            // Open cards can't be answered any more.
            let open: Vec<u64> = v
                .transcript
                .since(0)
                .into_iter()
                .filter(|i| matches!(i.body, ItemBody::Permission { answer: None, .. }))
                .map(|i| i.id)
                .collect();
            for id in open {
                if let Some(item) = v.transcript.update(id, |b| {
                    if let ItemBody::Permission { answer, .. } = b {
                        *answer = Some(PermissionAnswer::Cancelled);
                    }
                }) {
                    changed.push(item);
                }
            }
            if let Some(r) = &reason {
                changed.push(v.transcript.push(ItemBody::Error { message: r.clone() }));
            }
            v.status = ChatStatus::Stopped;
        }
        for item in changed {
            self.emit(ChatEventBody::Item {
                item: Box::new(item),
            });
        }
        // A newer session owns the thread's status once this one is replaced.
        if self.current.load(std::sync::atomic::Ordering::SeqCst) {
            self.host.interrupted(&self.thread).await;
        }
        self.emit(ChatEventBody::Status {
            status: ChatStatus::Stopped,
        });
        self.emit(ChatEventBody::Closed { reason });
    }
}

fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// `line` is 1-based; `limit` caps the number of lines.
fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_string();
    }
    let skip = line.map(|l| l.saturating_sub(1) as usize).unwrap_or(0);
    let take = limit.map(|l| l as usize).unwrap_or(usize::MAX);
    text.split_inclusive('\n').skip(skip).take(take).collect()
}

#[cfg(test)]
mod tests {
    use super::slice_lines;

    #[test]
    fn slices_by_line_and_limit() {
        let t = "a\nb\nc\n";
        assert_eq!(slice_lines(t, None, None), t);
        assert_eq!(slice_lines(t, Some(2), None), "b\nc\n");
        assert_eq!(slice_lines(t, Some(1), Some(2)), "a\nb\n");
        assert_eq!(slice_lines(t, Some(9), None), "");
    }
}
