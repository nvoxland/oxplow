//! One ACP agent session: an actor task that owns the connection and
//! turns protocol traffic into transcript items, policy decisions and
//! recording. The manager (`manager.rs`) holds only its command sender
//! and the shared [`SessionView`].
//!
//! The rules it keeps (see `.context/agent-model.md` → "ACP agents"):
//! - it sends a prompt only for [`Command::Prompt`], which comes only from
//!   a person's submit; a second prompt while a turn runs is an error,
//!   never queued;
//! - a tool call the policy denies is rejected without asking; anything
//!   else waits on the person, with no timeout and no "always allow" for
//!   writes;
//! - the turn-end directive is shown, never sent;
//! - a `session/load` replay rebuilds the transcript and records nothing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{IntentKind, PolicyDecision};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::warn;

use super::host::AcpHost;
use super::mapping::{self, AcpIntent};
use super::model::{
    AcpUpdate, ContextUsage, PermissionAnswer, PermissionKind, PermissionOption, ToolCall,
    ToolStatus,
};
use super::transcript::{ItemBody, Transcript, TranscriptItem};
use super::wire::{AgentConn, Incoming, McpHttp, PermissionReply, TurnEnd};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AcpStatus {
    Starting,
    Idle,
    Running,
    AwaitingPermission,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpEventBody {
    Item { item: Box<TranscriptItem> },
    Status { status: AcpStatus },
    Directive { text: Option<String> },
    Usage { usage: ContextUsage },
    Closed { reason: Option<String> },
}

/// A change to one thread's ACP session, pushed to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AcpEvent {
    pub thread_id: String,
    #[serde(flatten)]
    pub body: AcpEventBody,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AcpError {
    #[error("no ACP session is open for this thread")]
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

/// How to start the session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub thread_id: ThreadId,
    /// The `acpAgents` name.
    pub agent: String,
    pub cwd: PathBuf,
    pub mcp: Vec<McpHttp>,
    /// The thread's last session, loaded instead of starting fresh when
    /// the agent supports `session/load`.
    pub resume_session_id: Option<String>,
    pub system_prompt: Option<String>,
    /// Pass the system prompt as `_meta.systemPrompt.append` on
    /// `session/new` (the Claude adapter) instead of ahead of the first
    /// prompt.
    pub system_prompt_via_meta: bool,
}

/// What the manager and the UI read without a round trip to the actor.
#[derive(Debug)]
pub struct SessionView {
    pub agent: String,
    pub status: AcpStatus,
    pub transcript: Transcript,
    pub directive: Option<String>,
    pub session_id: Option<String>,
    /// The agent's last stderr lines, for a failed start or a crash.
    pub stderr_tail: Vec<String>,
}

pub const STDERR_TAIL: usize = 64;

impl SessionView {
    pub fn new(agent: &str) -> Self {
        Self {
            agent: agent.to_string(),
            status: AcpStatus::Starting,
            transcript: Transcript::default(),
            directive: None,
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
        reply: oneshot::Sender<Result<(), AcpError>>,
    },
    Cancel,
    Respond {
        request_id: String,
        option_id: Option<String>,
        reply: oneshot::Sender<Result<(), AcpError>>,
    },
    Close,
}

struct Pending {
    reply: PermissionReply,
    item_id: u64,
    options: Vec<PermissionOption>,
}

pub(super) struct Actor {
    spec: SessionSpec,
    host: Arc<dyn AcpHost>,
    view: Arc<Mutex<SessionView>>,
    events: broadcast::Sender<AcpEvent>,
    session_id: String,
    /// The prompt of the turn in flight.
    turn: Option<String>,
    pending: HashMap<String, Pending>,
    next_request: u64,
    /// Tool calls that went through a permission request.
    asked: HashSet<String>,
    /// Tool calls already recorded as finished.
    recorded: HashSet<String>,
    /// Paths written through `fs/write_text_file` this turn.
    fs_written: HashSet<String>,
    /// Post-tool nudges waiting for the person's next prompt.
    nudges: Vec<String>,
    /// The system prompt, when it rides the first prompt.
    pending_system_prompt: Option<String>,
    replaying: bool,
}

impl Actor {
    pub(super) fn new(
        spec: SessionSpec,
        host: Arc<dyn AcpHost>,
        view: Arc<Mutex<SessionView>>,
        events: broadcast::Sender<AcpEvent>,
    ) -> Self {
        Self {
            spec,
            host,
            view,
            events,
            session_id: String::new(),
            turn: None,
            pending: HashMap::new(),
            next_request: 0,
            asked: HashSet::new(),
            recorded: HashSet::new(),
            fs_written: HashSet::new(),
            nudges: Vec::new(),
            pending_system_prompt: None,
            replaying: false,
        }
    }

    fn emit(&self, body: AcpEventBody) {
        let _ = self.events.send(AcpEvent {
            thread_id: self.spec.thread_id.to_string(),
            body,
        });
    }

    fn push(&self, body: ItemBody) -> TranscriptItem {
        let item = self.view.lock().transcript.push(body);
        self.emit(AcpEventBody::Item {
            item: Box::new(item.clone()),
        });
        item
    }

    fn set_status(&self, status: AcpStatus) {
        let changed = {
            let mut v = self.view.lock();
            let changed = v.status != status;
            v.status = status;
            changed
        };
        if changed {
            self.emit(AcpEventBody::Status { status });
        }
    }

    fn thread(&self) -> &ThreadId {
        &self.spec.thread_id
    }

    /// Run the session until it is closed or the agent goes away.
    /// `ready` learns whether it started.
    pub(super) async fn run(
        mut self,
        conn: AgentConn,
        mut incoming: mpsc::UnboundedReceiver<Incoming>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        ready: oneshot::Sender<Result<(), AcpError>>,
    ) {
        if let Err(e) = self.start(&conn, &mut incoming).await {
            self.push(ItemBody::Error { message: e.clone() });
            self.set_status(AcpStatus::Stopped);
            self.emit(AcpEventBody::Closed {
                reason: Some(e.clone()),
            });
            let _ = ready.send(Err(AcpError::Agent(e)));
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
                    Some(cmd) => self.on_command(&conn, cmd).await,
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
        conn: &AgentConn,
        incoming: &mut mpsc::UnboundedReceiver<Incoming>,
    ) -> Result<(), String> {
        let info = conn.initialize().await?;
        if !self.spec.mcp.is_empty() && !info.mcp_http {
            return Err(format!(
                "ACP agent '{}' can't use HTTP MCP servers, and oxplow's tools reach ACP agents over HTTP MCP",
                self.spec.agent
            ));
        }
        let cwd = self.spec.cwd.clone();
        let mut loaded = None;
        if let Some(id) = self
            .spec
            .resume_session_id
            .clone()
            .filter(|s| !s.is_empty())
        {
            if info.load_session {
                self.replaying = true;
                let r = conn.load_session(&id, &cwd, &self.spec.mcp).await;
                while let Ok(msg) = incoming.try_recv() {
                    self.on_incoming(msg).await;
                }
                self.replaying = false;
                match r {
                    Ok(()) => loaded = Some(id),
                    Err(e) => warn!(error = %e, "acp: session/load failed; starting a new session"),
                }
            }
        }
        let session_id = match loaded {
            Some(id) => id,
            None => {
                let meta = match (&self.spec.system_prompt, self.spec.system_prompt_via_meta) {
                    (Some(sp), true) => Some(serde_json::json!({"systemPrompt": {"append": sp}})),
                    _ => None,
                };
                if !self.spec.system_prompt_via_meta {
                    self.pending_system_prompt = self.spec.system_prompt.clone();
                }
                conn.new_session(&cwd, &self.spec.mcp, meta).await?
            }
        };
        self.session_id = session_id.clone();
        self.view.lock().session_id = Some(session_id.clone());
        self.host.session_started(self.thread(), &session_id).await;
        self.set_status(AcpStatus::Idle);
        Ok(())
    }

    async fn on_command(&mut self, conn: &AgentConn, cmd: Command) {
        match cmd {
            Command::Prompt { text, reply } => {
                let r = self.prompt(conn, text).await;
                let _ = reply.send(r);
            }
            Command::Cancel => {
                if self.turn.is_some() {
                    conn.cancel(&self.session_id);
                }
                // The protocol wants every open permission request
                // answered `cancelled` once the client cancels.
                if !self.pending.is_empty() {
                    self.cancel_pending();
                    self.host.awaiting_user(self.thread(), None).await;
                }
            }
            Command::Respond {
                request_id,
                option_id,
                reply,
            } => {
                let _ = reply.send(self.respond(&request_id, option_id).await);
            }
            Command::Close => {}
        }
    }

    /// The person's prompt: the one place a prompt is built and sent.
    async fn prompt(&mut self, conn: &AgentConn, text: String) -> Result<(), AcpError> {
        if self.turn.is_some() {
            return Err(AcpError::TurnInFlight);
        }
        if text.trim().is_empty() {
            return Err(AcpError::EmptyPrompt);
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
        parts.append(&mut self.nudges);
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
        if let Err(e) = conn.prompt(&self.session_id, hp) {
            self.turn = None;
            self.push(ItemBody::Error { message: e.clone() });
            return Err(AcpError::Agent(e));
        }
        self.set_status(AcpStatus::Running);
        Ok(())
    }

    async fn respond(
        &mut self,
        request_id: &str,
        option_id: Option<String>,
    ) -> Result<(), AcpError> {
        let Some(p) = self.pending.remove(request_id) else {
            return Err(AcpError::UnknownRequest(request_id.to_string()));
        };
        let answer = match option_id {
            Some(id) if p.options.iter().any(|o| o.id == id) => {
                PermissionAnswer::Selected { option_id: id }
            }
            Some(id) => {
                self.pending.insert(request_id.to_string(), p);
                return Err(AcpError::UnknownOption(id));
            }
            None => PermissionAnswer::Cancelled,
        };
        p.reply.answer(&answer);
        self.mark_answered(p.item_id, answer);
        if self.pending.is_empty() {
            self.host.awaiting_user(self.thread(), None).await;
            if self.turn.is_some() {
                self.set_status(AcpStatus::Running);
            }
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
            self.emit(AcpEventBody::Item {
                item: Box::new(item),
            });
        }
    }

    fn cancel_pending(&mut self) {
        for (_, p) in self.pending.drain().collect::<Vec<_>>() {
            p.reply.answer(&PermissionAnswer::Cancelled);
            self.mark_answered(p.item_id, PermissionAnswer::Cancelled);
        }
    }

    async fn on_incoming(&mut self, msg: Incoming) {
        if !self.replaying {
            self.host.activity(self.thread());
        }
        match msg {
            Incoming::Update { update, .. } => self.on_update(update).await,
            Incoming::Permission { ask, reply, .. } => {
                let ask = *ask;
                if self.replaying {
                    reply.answer(&PermissionAnswer::Cancelled);
                    return;
                }
                let tool_id = ask.tool.id.clone();
                self.apply(AcpUpdate::ToolCallUpdate(ask.tool.clone()));
                let tool = self
                    .view
                    .lock()
                    .transcript
                    .tool(&tool_id)
                    .cloned()
                    .unwrap_or_else(|| ToolCall::from_patch(&ask.tool));
                self.asked.insert(tool_id.clone());
                let intent = mapping::intent_for(&tool);
                let payload = self.payload(&tool, &intent);
                match self
                    .host
                    .check_tool(self.thread(), &self.session_id, &intent, &payload)
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
                        self.push(ItemBody::PolicyDenied {
                            tool_call_id: tool_id,
                            label: intent.label,
                            reason,
                        });
                    }
                    PolicyDecision::Allow => {
                        let write = intent.kind == IntentKind::WorktreeWrite;
                        let options: Vec<PermissionOption> = ask
                            .options
                            .into_iter()
                            .filter(|o| !(write && o.kind == PermissionKind::AllowAlways))
                            .collect();
                        self.next_request += 1;
                        let request_id = format!("perm-{}", self.next_request);
                        let title = if tool.title.is_empty() {
                            intent.label.clone()
                        } else {
                            tool.title.clone()
                        };
                        let item = self.push(ItemBody::Permission {
                            request_id: request_id.clone(),
                            tool_call_id: tool_id,
                            title: title.clone(),
                            options: options.clone(),
                            answer: None,
                        });
                        self.pending.insert(
                            request_id,
                            Pending {
                                reply,
                                item_id: item.id,
                                options,
                            },
                        );
                        self.set_status(AcpStatus::AwaitingPermission);
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
                ..
            } => {
                let path = self.absolute(&path);
                let p = path.to_string_lossy().into_owned();
                let intent = AcpIntent {
                    label: "Write".into(),
                    kind: IntentKind::WorktreeWrite,
                    paths: vec![p.clone()],
                };
                let payload =
                    serde_json::json!({"tool_name": "Write", "tool_input": {"file_path": p}});
                let decision = if self.replaying {
                    PolicyDecision::Deny {
                        layer: oxplow_runtime::policy::DenyLayer::WriteGuard,
                        reason: "oxplow is replaying this session; nothing is written".into(),
                    }
                } else {
                    self.host
                        .check_tool(self.thread(), &self.session_id, &intent, &payload)
                        .await
                };
                match decision {
                    PolicyDecision::Deny { reason, .. } => {
                        reply.deny(&reason);
                        if !self.replaying {
                            self.push(ItemBody::PolicyDenied {
                                tool_call_id: String::new(),
                                label: "Write".into(),
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
                ..
            } => {
                let path = self.absolute(&path);
                match std::fs::read_to_string(&path) {
                    Ok(text) => reply.ok(slice_lines(&text, line, limit)),
                    Err(e) => reply.err(&format!("reading {}: {e}", path.display())),
                }
            }
            Incoming::PromptDone(result) => self.on_turn_end(result).await,
        }
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.spec.cwd.join(path)
        }
    }

    fn payload(&self, tool: &ToolCall, intent: &AcpIntent) -> serde_json::Value {
        mapping::canonical_events(tool, Some(&self.session_id))
            .first()
            .map(|e| e.to_payload())
            .unwrap_or_else(|| serde_json::json!({"tool_name": intent.label, "tool_input": {}}))
    }

    fn apply(&self, update: AcpUpdate) -> Option<TranscriptItem> {
        let usage = matches!(update, AcpUpdate::Usage(_));
        let item = self.view.lock().transcript.apply(update);
        if let Some(item) = &item {
            self.emit(AcpEventBody::Item {
            item: Box::new(item.clone()),
        });
        }
        if usage {
            if let Some(u) = self.view.lock().transcript.usage().cloned() {
                self.emit(AcpEventBody::Usage { usage: u });
            }
        }
        item
    }

    async fn on_update(&mut self, update: AcpUpdate) {
        let tool_id = match &update {
            AcpUpdate::ToolCall(t) => Some(t.id.clone()),
            AcpUpdate::ToolCallUpdate(p) => Some(p.id.clone()),
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
        // A write that finished without asking and without going through
        // fs/write_text_file slipped past the gate: say so if the policy
        // would have refused it.
        if mapping::is_write(tool.kind)
            && tool.status == ToolStatus::Completed
            && !self.asked.contains(&id)
        {
            let intent = mapping::intent_for(&tool);
            let unseen = intent.paths.is_empty()
                || intent.paths.iter().any(|p| !self.fs_written.contains(p));
            if unseen {
                let payload = self.payload(&tool, &intent);
                if let PolicyDecision::Deny { reason, .. } = self
                    .host
                    .check_tool(self.thread(), &self.session_id, &intent, &payload)
                    .await
                {
                    self.push(ItemBody::Bypass {
                        tool_call_id: id.clone(),
                        label: intent.label,
                        reason,
                    });
                }
            }
        }
        for ev in mapping::canonical_events(&tool, Some(&self.session_id)) {
            if let Some(n) = self
                .host
                .tool_finished(self.thread(), &self.session_id, &ev)
                .await
            {
                self.nudges.push(n);
            }
        }
    }

    async fn on_turn_end(&mut self, result: Result<TurnEnd, String>) {
        let prompt = self.turn.take().unwrap_or_default();
        // Clear "awaiting you" BEFORE the Stop is ingested: Stop keeps an
        // AwaitingUser status (for `await_user`), which would strand it.
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
        let directive = self
            .host
            .turn_ended(self.thread(), &self.session_id, &prompt, tokens.as_ref())
            .await;
        if let Some(text) = directive {
            self.view.lock().directive = Some(text.clone());
            self.push(ItemBody::Directive { text: text.clone() });
            self.emit(AcpEventBody::Directive { text: Some(text) });
        }
        self.set_status(AcpStatus::Idle);
    }

    async fn shut_down(&mut self, reason: Option<String>) {
        // Answers can't reach an agent that's gone; mark the cards.
        for (_, p) in self.pending.drain().collect::<Vec<_>>() {
            self.mark_answered(p.item_id, PermissionAnswer::Cancelled);
        }
        if let Some(r) = &reason {
            self.push(ItemBody::Error { message: r.clone() });
        }
        self.turn = None;
        self.host.interrupted(self.thread()).await;
        self.set_status(AcpStatus::Stopped);
        self.emit(AcpEventBody::Closed { reason });
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
