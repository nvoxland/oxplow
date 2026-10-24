//! oxplow's observer of a Codex session's thread on Codex's app-server
//! daemon (`.context/agent-model.md` "Codex: its TUI on a thread oxplow
//! observes"). The harness says how the thread opens
//! (`LaunchSpec::CodexAppServer`); this opens it over its own connection
//! to the daemon — before Codex's TUI attaches, so the thread keeps the
//! MCP server and system prompt oxplow gave it — and maps its
//! notifications into [`Incoming`] messages. It never sends a turn: the
//! person types into the TUI, and every prompt arrives as the thread's
//! user message, which the session records as theirs. Approvals are the
//! TUI's to show; this answers no request.
//!
//! The daemon sends a client what concerns every thread it subscribed to
//! and some that it didn't (Codex's title-generation thread), so only
//! messages naming this session's thread pass.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::hook::TurnRun;
use oxplow_harnesses::codex_app_server::{AppServer, ServerMessage};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use super::conn::{cap, cap_value};
use super::conn::{Connection, Incoming, Opened, SubagentChange, TurnEnd, TurnTokens};
use super::human_prompt::HumanPrompt;
use super::model::{
    ChatUpdate, ContextUsage, PlanEntry, PlanStatus, ToolCall, ToolKind, ToolStatus,
};

/// How the thread opens, as the harness's launch says.
#[derive(Debug, Clone)]
pub struct CodexOpen {
    pub socket: PathBuf,
    pub program: Option<PathBuf>,
    pub method: String,
    pub params: Value,
}

/// What a new thread's goal is set to, and cleared from, to write its
/// rollout: a thread nothing has persisted can't be resumed, and Codex's
/// TUI attaches by resuming.
const PERSIST_GOAL: &str = "oxplow opened this thread";

#[derive(Default)]
struct State {
    /// The thread's id, once open: the filter on what passes.
    thread: Option<String>,
    /// Agent messages and reasoning that arrived as deltas: their
    /// completed item repeats the text.
    streamed: HashSet<String>,
    /// The thread's cumulative counts at the last turn's end, and now.
    tokens_at_turn_start: Option<Counts>,
    tokens_now: Option<Counts>,
    /// What the last approval or question request this client saw asked,
    /// for the wait that follows it. The request is the TUI's to answer;
    /// it is only read.
    asked: Option<String>,
    /// The thread is waiting on the person (the status was reported).
    waiting: bool,
    /// The child agents this thread drives (Codex's collab agents), by
    /// their own thread's id: what passes of what their threads send.
    children: std::collections::HashMap<String, Child>,
    /// The model and effort the thread has now: what it opened on, then
    /// what its settings changes made them; and the context fill its last
    /// `thread/tokenUsage/updated` said.
    run: TurnRun,
    /// A turn of the thread is running (`turn/started` to `turn/completed`).
    running: bool,
    /// The daemon doesn't know `thread/inject_items`.
    inject_unsupported: bool,
}

/// One child agent, as far as it has been seen.
#[derive(Default)]
struct Child {
    /// Its role or path, when Codex said.
    kind: Option<String>,
    started: bool,
    finished: bool,
}

/// The model and effort a thread opened on (`thread/start`, `resume` or
/// `fork`'s result).
fn thread_run(opened: &Value) -> TurnRun {
    let text = |k: &str| {
        opened
            .pointer(&format!("/thread/{k}"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    TurnRun {
        model: text("model"),
        effort: text("reasoningEffort"),
        ..TurnRun::default()
    }
}

/// One `tokenUsage` breakdown.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Counts {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
}

impl Counts {
    fn of(v: &Value) -> Option<Self> {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        v.is_object().then(|| Counts {
            input: n("inputTokens"),
            cached: n("cachedInputTokens"),
            cache_write: n("cacheWriteInputTokens"),
            output: n("outputTokens"),
        })
    }

    fn since(self, earlier: Counts) -> Counts {
        Counts {
            input: self.input.saturating_sub(earlier.input),
            cached: self.cached.saturating_sub(earlier.cached),
            cache_write: self.cache_write.saturating_sub(earlier.cache_write),
            output: self.output.saturating_sub(earlier.output),
        }
    }

    fn minus_last(self, last: Option<Counts>) -> Counts {
        last.map_or(self, |l| self.since(l))
    }

    /// As a turn's tokens: input is what wasn't read from the cache.
    fn tokens(self) -> TurnTokens {
        TurnTokens {
            input: self.input.saturating_sub(self.cached),
            output: self.output,
            cache_read: self.cached,
            cache_write: self.cache_write,
        }
    }
}

pub struct CodexConnection {
    server: AppServer,
    open: CodexOpen,
    tx: mpsc::UnboundedSender<Incoming>,
    state: Arc<Mutex<State>>,
    /// Set once the thread is open: the pump holds what arrives until it
    /// knows the thread's id.
    opened: watch::Sender<bool>,
}

impl CodexConnection {
    /// Connect to the daemon (starting it when it isn't running) and
    /// start the pump that turns its messages into [`Incoming`] ones.
    pub async fn connect(
        open: CodexOpen,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Incoming>), String> {
        let (server, messages) = AppServer::connect(&open.socket, open.program.as_deref()).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (opened, opened_rx) = watch::channel(false);
        let conn = Arc::new(Self {
            server: server.clone(),
            open,
            tx: tx.clone(),
            state: Arc::default(),
            opened,
        });
        tokio::spawn(pump(messages, opened_rx, tx, conn.state.clone()));
        Ok((conn, rx))
    }

    fn thread(&self) -> String {
        self.state.lock().thread.clone().unwrap_or_default()
    }

    /// Write thread `thread`'s rollout: setting its goal does, and clearing
    /// it leaves none behind.
    async fn persist(&self, thread: &str) -> Result<(), String> {
        let persist = |params| self.server.request("thread/goal/set", params);
        persist(json!({"threadId": thread, "objective": PERSIST_GOAL}))
            .await
            .map_err(|e| format!("saving the new thread: {e}"))?;
        self.server
            .request("thread/goal/clear", json!({"threadId": thread}))
            .await
            .map_err(|e| format!("saving the new thread: {e}"))?;
        Ok(())
    }
}

#[async_trait]
impl Connection for CodexConnection {
    /// The harness's request (`thread/start`, `thread/resume`,
    /// `thread/fork`), sandboxed to the thread's worktree: Codex may change
    /// it and nothing outside it — another stream's worktree included
    /// (`.context/agent-model.md` "The sandbox confines Codex to its
    /// worktree"). A new thread is persisted at once, so Codex's TUI can
    /// resume it.
    async fn open(&self, _cwd: &Path) -> Result<Opened, String> {
        let mut params = self.open.params.clone();
        params["sandbox"] = json!("workspace-write");
        let opened = self
            .server
            .request(&self.open.method, params)
            .await
            .map_err(|e| format!("Codex refused to open the thread: {e}"))?;
        let thread = opened
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or("Codex opened no thread")?
            .to_string();
        if self.open.method == "thread/start" {
            self.persist(&thread).await?;
        }
        if let Some(name) = opened
            .pointer("/thread/name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
        {
            let _ = self.tx.send(Incoming::Named(name.to_string()));
        }
        {
            let mut state = self.state.lock();
            state.thread = Some(thread.clone());
            state.run = thread_run(&opened);
        }
        let _ = self.opened.send(true);
        Ok(Opened {
            session_id: thread,
            preamble: None,
        })
    }

    /// The thread's name, as Codex's own thread list shows it.
    async fn rename(&self, name: &str) -> Result<(), String> {
        self.server
            .request(
                "thread/name/set",
                json!({"threadId": self.thread(), "name": name}),
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("Codex didn't take the name: {e}"))
    }

    /// The person prompts in Codex's TUI; nothing here sends a turn.
    fn prompt(&self, _prompt: HumanPrompt) -> Result<(), String> {
        Err("a Codex session is prompted in Codex's own TUI".into())
    }

    /// The TUI stops its own turns.
    fn cancel(&self) {}

    /// The daemon takes `thread/inject_items`, and no turn is running.
    fn can_inject(&self) -> bool {
        let state = self.state.lock();
        !state.running && !state.inject_unsupported
    }

    /// A developer item in the thread's history: it starts no turn and
    /// spends no tokens by itself, answers no pending approval or question,
    /// and the agent reads it at its next turn (it survives a compaction
    /// and a resume). Sent only when no turn runs (`can_inject`): during
    /// one the daemon takes it as more input and the turn answers again.
    async fn inject(&self, text: &str) -> Result<(), String> {
        let item = json!({
            "type": "message",
            "role": "developer",
            "content": [{"type": "input_text", "text": text}],
        });
        self.server
            .request(
                "thread/inject_items",
                json!({"threadId": self.thread(), "items": [item]}),
            )
            .await
            .map(|_| ())
            .map_err(|e| {
                if inject_unsupported(&e) {
                    self.state.lock().inject_unsupported = true;
                }
                format!("Codex didn't take the context: {e}")
            })
    }

    async fn closed(&self) {
        self.server.closed().await;
    }
}

/// Whether a daemon's error says it has no `thread/inject_items`.
fn inject_unsupported(error: &str) -> bool {
    error.contains("unknown variant")
}

/// Every message for the thread, in order, once it is open.
async fn pump(
    mut messages: mpsc::UnboundedReceiver<ServerMessage>,
    mut opened: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<Incoming>,
    state: Arc<Mutex<State>>,
) {
    if opened.wait_for(|o| *o).await.is_err() {
        return;
    }
    while let Some(message) = messages.recv().await {
        for incoming in on_message(&message, &mut state.lock()) {
            if tx.send(incoming).is_err() {
                return;
            }
        }
    }
}

/// What one daemon message makes of the thread: its own messages, those of
/// a child agent it drives (only the child's finished tool calls), and the
/// start of a child thread it spawned.
fn on_message(message: &ServerMessage, state: &mut State) -> Vec<Incoming> {
    let Some(ours) = state.thread.clone() else {
        return Vec::new();
    };
    let thread = message.thread_id();
    if thread == Some(ours.as_str()) {
        // A request (an approval, a question) is the TUI's to answer; what
        // it asks is remembered for the wait the status reports.
        return match message {
            ServerMessage::Notification { method, params } => {
                on_notification(method, params, state)
            }
            ServerMessage::Request { method, params, .. } => {
                note_request(method, params, state);
                Vec::new()
            }
        };
    }
    if let ServerMessage::Notification { method, params } = message {
        if let Some(child) = thread.filter(|t| state.children.contains_key(*t)) {
            return on_child_notification(child, method, params, state);
        }
        if method == "thread/started" {
            // The daemon says a thread began, and whose child it is.
            let spawn = params.pointer("/thread/source/subAgent/thread_spawn");
            let parent = spawn.and_then(|s| s.get("parent_thread_id")?.as_str());
            if let (Some(id), true) = (
                params.pointer("/thread/id").and_then(Value::as_str),
                parent == Some(ours.as_str()),
            ) {
                let kind = ["agent_role", "agent_nickname"]
                    .iter()
                    .find_map(|k| spawn?.get(*k)?.as_str().filter(|s| !s.is_empty()))
                    .map(str::to_string);
                let child = state.children.entry(id.to_string()).or_default();
                child.kind = child.kind.take().or(kind);
            }
        }
    }
    Vec::new()
}

/// A child agent's finished tool call, as the child's. Its talk, tokens,
/// status and waits are its own thread's, not the parent's.
fn on_child_notification(child: &str, method: &str, p: &Value, state: &mut State) -> Vec<Incoming> {
    if method != "item/completed" {
        return Vec::new();
    }
    let Some(item) = p.get("item") else {
        return Vec::new();
    };
    let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    let ty = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if matches!(
        ty,
        "userMessage"
            | "agentMessage"
            | "reasoning"
            | "plan"
            | "hookPrompt"
            | "contextCompaction"
            | "enteredReviewMode"
            | "exitedReviewMode"
            | "sleep"
            | "subAgentActivity"
            | "collabAgentToolCall"
    ) {
        return Vec::new();
    }
    let Some(call) = tool_call(item, id.to_string(), true) else {
        return Vec::new();
    };
    let kind = state.children.get(child).and_then(|c| c.kind.clone());
    vec![Incoming::ChildCall {
        child: oxplow_domain::agent::tool::Subagent {
            id: child.to_string(),
            kind,
        },
        call: Box::new(call),
    }]
}

/// The lifecycle a child's state names, once each: `Started` the first
/// time it is seen, `Finished` when it ends.
fn child_changes(
    state: &mut State,
    id: &str,
    kind: Option<String>,
    status: &str,
    said: Option<String>,
) -> Vec<Incoming> {
    let child = state.children.entry(id.to_string()).or_default();
    child.kind = child.kind.take().or(kind);
    let mut out = Vec::new();
    let ended = matches!(
        status,
        "completed" | "interrupted" | "errored" | "shutdown" | "notFound"
    );
    if !child.started {
        child.started = true;
        out.push(Incoming::Subagent {
            change: SubagentChange::Started,
            id: id.to_string(),
            kind: child.kind.clone(),
        });
    }
    if ended && !child.finished {
        child.finished = true;
        out.push(Incoming::Subagent {
            change: SubagentChange::Finished(
                said.filter(|s| !s.trim().is_empty())
                    .or_else(|| Some(status.to_string())),
            ),
            id: id.to_string(),
            kind: child.kind.clone(),
        });
    }
    out
}

/// A collab call or a child's activity item, as the lifecycle of the
/// children it names.
fn collab_lifecycle(item: &Value, state: &mut State) -> Vec<Incoming> {
    match item.get("type").and_then(Value::as_str) {
        Some("collabAgentToolCall") => {
            let states = item.get("agentsStates").and_then(Value::as_object);
            let mut out = Vec::new();
            let ids = item
                .get("receiverThreadIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for id in ids {
                let s = states.and_then(|m| m.get(id));
                let status = s
                    .and_then(|s| s.get("status")?.as_str())
                    .unwrap_or("pendingInit");
                // `notFound` names an id that never was a child.
                if status == "notFound" && !state.children.contains_key(id) {
                    continue;
                }
                let said = s
                    .and_then(|s| s.get("message")?.as_str())
                    .map(str::to_string);
                out.extend(child_changes(state, id, None, status, said));
            }
            out
        }
        Some("subAgentActivity") => {
            let Some(id) = item.get("agentThreadId").and_then(Value::as_str) else {
                return Vec::new();
            };
            let kind = item
                .get("agentPath")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string);
            let status = match item.get("kind").and_then(Value::as_str) {
                Some("completed") => "completed",
                Some("interrupted") => "interrupted",
                // Started, or talked to: it is running.
                _ => "running",
            };
            child_changes(state, id, kind, status, None)
        }
        _ => Vec::new(),
    }
}

/// Remember what an approval or question request asks, to say it when the
/// thread's status reports the wait.
fn note_request(method: &str, p: &Value, state: &mut State) {
    let text = |pointer: &str| {
        p.pointer(pointer)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    };
    state.asked = match method {
        "item/commandExecution/requestApproval" => {
            text("/command").map(|c| format!("Approve in Codex: {c}"))
        }
        "item/fileChange/requestApproval" | "item/permissions/requestApproval" => {
            text("/reason").map(|r| format!("Approve in Codex: {r}"))
        }
        "item/tool/requestUserInput" => text("/questions/0/question").map(str::to_string),
        _ => return,
    };
}

fn on_notification(method: &str, p: &Value, state: &mut State) -> Vec<Incoming> {
    let update = |u: ChatUpdate| vec![Incoming::Update(Box::new(u))];
    match method {
        // The thread waits on the person, or no longer does.
        "thread/status/changed" => {
            let flags: Vec<&str> = p
                .pointer("/status/activeFlags")
                .and_then(Value::as_array)
                .map(|f| f.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let on_approval = flags.contains(&"waitingOnApproval");
            let on_input = flags.contains(&"waitingOnUserInput");
            if on_approval || on_input {
                if state.waiting {
                    return Vec::new();
                }
                state.waiting = true;
                let said = state.asked.take().unwrap_or_else(|| {
                    if on_input {
                        "Waiting on your answer in Codex".to_string()
                    } else {
                        "Waiting on an approval in Codex".to_string()
                    }
                });
                vec![Incoming::Waiting(Some(said))]
            } else {
                state.asked = None;
                if std::mem::take(&mut state.waiting) {
                    vec![Incoming::Waiting(None)]
                } else {
                    Vec::new()
                }
            }
        }
        // Answered, one way or another.
        "serverRequest/resolved" => {
            state.asked = None;
            if std::mem::take(&mut state.waiting) {
                vec![Incoming::Waiting(None)]
            } else {
                Vec::new()
            }
        }
        // The plan the agent put to the person: the completed item is the
        // whole text.
        "item/completed" if p.pointer("/item/type").and_then(Value::as_str) == Some("plan") => {
            let text = p
                .pointer("/item/text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![Incoming::Plan(text.to_string())]
            }
        }
        // The thread's context was summarized. Codex has no prompt channel,
        // so nothing of oxplow's own is re-sent after it.
        "item/completed"
            if p.pointer("/item/type").and_then(Value::as_str) == Some("contextCompaction") =>
        {
            vec![Incoming::Compacted]
        }
        "item/started" | "item/completed" => {
            let Some(item) = p.get("item") else {
                return Vec::new();
            };
            let mut out: Vec<Incoming> =
                item_updates(item, method == "item/completed", &mut state.streamed)
                    .into_iter()
                    .map(|u| Incoming::Update(Box::new(u)))
                    .collect();
            out.extend(collab_lifecycle(item, state));
            out
        }
        "item/agentMessage/delta" => {
            streamed(p, state);
            update(ChatUpdate::AgentChunk(text_of(p, "delta")))
        }
        "item/reasoning/summaryTextDelta" => {
            streamed(p, state);
            update(ChatUpdate::ThoughtChunk(text_of(p, "delta")))
        }
        "turn/plan/updated" => update(ChatUpdate::Plan(plan(p))),
        "thread/tokenUsage/updated" => {
            let total = p.pointer("/tokenUsage/total").and_then(Counts::of);
            let last = p.pointer("/tokenUsage/last").and_then(Counts::of);
            if let (None, Some(total)) = (state.tokens_at_turn_start, total) {
                // The first reading since the thread opened: the turn's
                // start is before its last request.
                state.tokens_at_turn_start = Some(total.minus_last(last));
            }
            state.tokens_now = total.or(state.tokens_now);
            let used = p
                .pointer("/tokenUsage/last/totalTokens")
                .and_then(Value::as_u64);
            let size = p
                .pointer("/tokenUsage/modelContextWindow")
                .and_then(Value::as_u64);
            state.run.context_tokens = used.and_then(|n| i64::try_from(n).ok());
            state.run.context_window = size.and_then(|n| i64::try_from(n).ok());
            match (used, size) {
                (Some(used), Some(size)) => update(ChatUpdate::Usage(ContextUsage {
                    used,
                    size,
                    cost_amount: None,
                    cost_currency: None,
                })),
                _ => Vec::new(),
            }
        }
        "turn/started" => {
            state.running = true;
            Vec::new()
        }
        "turn/completed" => {
            state.running = false;
            let tokens = match (state.tokens_now, state.tokens_at_turn_start) {
                (Some(now), Some(start)) => Some(now.since(start).tokens()),
                _ => None,
            };
            state.tokens_at_turn_start = state.tokens_now;
            let status = p
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let done = match status {
                "failed" => Err(p
                    .pointer("/turn/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the turn failed")
                    .to_string()),
                _ => Ok(TurnEnd {
                    stop_reason: status.to_string(),
                    usage: tokens,
                    run: (state.run != TurnRun::default()).then(|| state.run.clone()),
                }),
            };
            vec![Incoming::PromptDone(done)]
        }
        "thread/settings/updated" => {
            let setting = |k: &str| {
                p.pointer(&format!("/threadSettings/{k}"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            state.run.model = setting("model").or(state.run.model.take());
            state.run.effort = setting("effort").or(state.run.effort.take());
            Vec::new()
        }
        "thread/name/updated" => p
            .get("threadName")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(|n| vec![Incoming::Named(n.to_string())])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn streamed(p: &Value, state: &mut State) {
    if let Some(id) = p.get("itemId").and_then(Value::as_str) {
        state.streamed.insert(id.to_string());
    }
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(cap)
        .unwrap_or_default()
}

fn plan(p: &Value) -> Vec<PlanEntry> {
    p.get("plan")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|step| PlanEntry {
            content: text_of(step, "step"),
            status: match step.get("status").and_then(Value::as_str) {
                Some("completed") => PlanStatus::Completed,
                Some("inProgress") => PlanStatus::InProgress,
                _ => PlanStatus::Pending,
            },
        })
        .collect()
}

/// What one thread item says, started or `done`. Text that arrived as
/// deltas (its id in `streamed`) isn't repeated when the item completes.
fn item_updates(item: &Value, done: bool, streamed: &mut HashSet<String>) -> Vec<ChatUpdate> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let was_streamed = done && streamed.remove(&id);
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "userMessage" if done => {
            let text: Vec<String> = item
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .map(cap)
                .collect();
            vec![ChatUpdate::UserChunk(text.join("\n"))]
        }
        "agentMessage" if done && !was_streamed => {
            vec![ChatUpdate::AgentChunk(text_of(item, "text"))]
        }
        "reasoning" if done && !was_streamed => {
            let summary: Vec<String> = item
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(cap)
                .collect();
            if summary.is_empty() {
                Vec::new()
            } else {
                vec![ChatUpdate::ThoughtChunk(summary.join("\n"))]
            }
        }
        "userMessage" | "agentMessage" | "reasoning" | "plan" | "hookPrompt"
        | "contextCompaction" | "enteredReviewMode" | "exitedReviewMode" | "sleep"
        | "subAgentActivity" => Vec::new(),
        _ => tool_call(item, id, done)
            .into_iter()
            .map(ChatUpdate::ToolCall)
            .collect(),
    }
}

/// A tool item as a tool call: its kind, title, input and (once it ends)
/// output, in the protocol-neutral shape the session maps and records.
/// An item without a status of its own (a web search) is running until
/// it is `done`.
fn tool_call(item: &Value, id: String, done: bool) -> Option<ToolCall> {
    let s = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default();
    let status = match s("status") {
        "completed" => ToolStatus::Completed,
        "failed" | "declined" => ToolStatus::Failed,
        "inProgress" => ToolStatus::InProgress,
        _ if done => ToolStatus::Completed,
        _ => ToolStatus::InProgress,
    };
    let mut call = ToolCall {
        id,
        status,
        ..ToolCall::default()
    };
    match s("type") {
        "commandExecution" => {
            let actions = item
                .get("commandActions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let action_type = |a: &Value| a.get("type").and_then(Value::as_str).map(str::to_string);
            let all = |t: &str| {
                !actions.is_empty() && actions.iter().all(|a| action_type(a).as_deref() == Some(t))
            };
            call.kind = if all("read") {
                ToolKind::Read
            } else if actions
                .iter()
                .all(|a| matches!(action_type(a).as_deref(), Some("search" | "listFiles")))
                && !actions.is_empty()
            {
                ToolKind::Search
            } else {
                ToolKind::Execute
            };
            call.locations = actions
                .iter()
                .filter_map(|a| a.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            // The command as the model wrote it, not its shell wrapper.
            let command = actions
                .iter()
                .filter_map(|a| a.get("command").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" && ");
            let command = if command.is_empty() {
                s("command").to_string()
            } else {
                command
            };
            call.title = command.clone();
            call.raw_input = Some(json!({"command": command, "cwd": item.get("cwd")}));
            if status != ToolStatus::InProgress {
                call.raw_output = Some(json!({"exit_code": item.get("exitCode")}));
                if let Some(out) = item.get("aggregatedOutput").and_then(Value::as_str) {
                    call.text = vec![cap(out)];
                }
            }
        }
        "fileChange" => {
            let changes = item
                .get("changes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let kind_of = |c: &Value| {
                c.pointer("/kind/type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            call.kind = if changes
                .iter()
                .all(|c| kind_of(c).as_deref() == Some("delete"))
                && !changes.is_empty()
            {
                ToolKind::Delete
            } else if changes
                .iter()
                .any(|c| c.pointer("/kind/move_path").is_some_and(|m| !m.is_null()))
            {
                ToolKind::Move
            } else {
                ToolKind::Edit
            };
            call.locations = changes
                .iter()
                .flat_map(|c| {
                    [c.get("path"), c.pointer("/kind/move_path")]
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                })
                .collect();
            call.title = format!("Edit {}", call.locations.join(", "));
            call.raw_input = Some(json!({"changes": changes.iter().map(|c| json!({
                "path": c.get("path"), "kind": kind_of(c),
            })).collect::<Vec<_>>()}));
            call.text = changes
                .iter()
                .filter_map(|c| c.get("diff").and_then(Value::as_str))
                .map(cap)
                .collect();
        }
        "mcpToolCall" => {
            call.name = Some(format!("mcp__{}__{}", s("server"), s("tool")));
            call.title = format!("{}.{}", s("server"), s("tool"));
            call.raw_input = item.get("arguments").cloned();
            if let Some(e) = item.get("error").filter(|e| !e.is_null()) {
                call.status = ToolStatus::Failed;
                call.raw_output = Some(cap_value(e.clone()));
            } else if let Some(r) = item.get("result").filter(|r| !r.is_null()) {
                call.raw_output = Some(cap_value(r.clone()));
            }
        }
        "collabAgentToolCall" => {
            call.kind = ToolKind::Subagent;
            call.name = Some(s("tool").to_string());
            call.title = format!("Agent: {}", s("tool"));
            call.raw_input = Some(json!({
                "tool": item.get("tool"),
                "prompt": item.get("prompt").map(|p| p.as_str().map_or(Value::Null, |p| json!(cap(p)))),
                "receiverThreadIds": item.get("receiverThreadIds"),
            }));
            if status != ToolStatus::InProgress {
                call.raw_output =
                    Some(cap_value(json!({"agentsStates": item.get("agentsStates")})));
            }
        }
        "webSearch" => {
            call.kind = ToolKind::Fetch;
            call.title = format!("Search the web: {}", s("query"));
            call.raw_input = Some(json!({"query": item.get("query")}));
        }
        "dynamicToolCall" => {
            call.title = s("tool").to_string();
            call.raw_input = item.get("arguments").cloned();
        }
        "imageView" => {
            call.kind = ToolKind::Read;
            call.title = format!("View {}", s("path"));
            call.locations = vec![s("path").to_string()];
        }
        other => {
            call.title = other.to_string();
            call.raw_input = Some(cap_value(item.clone()));
        }
    }
    Some(call)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded thread (`tests/fixtures/codex/app-server.jsonl`, Codex
    /// 0.163.0): a person's "say hi", then a shell command appending to
    /// `notes.txt`, then a direct edit of it, then a rename.
    fn recorded() -> Vec<ServerMessage> {
        include_str!("../../tests/fixtures/codex/app-server.jsonl")
            .lines()
            .skip(1)
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| {
                let method = v.get("method")?.as_str()?.to_string();
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                Some(match v.get("id") {
                    Some(id) => ServerMessage::Request {
                        id: id.clone(),
                        method,
                        params,
                    },
                    None => ServerMessage::Notification { method, params },
                })
            })
            .collect()
    }

    const THREAD: &str = "01a14232-9ba1-7d40-8466-1ec0e927ef44";

    /// The recorded thread's notifications, as the session gets them.
    fn mapped() -> Vec<Incoming> {
        let mut state = State {
            thread: Some(THREAD.into()),
            ..State::default()
        };
        recorded()
            .into_iter()
            .filter(|m| m.thread_id() == Some(THREAD))
            .flat_map(|m| match m {
                ServerMessage::Notification { method, params } => {
                    on_notification(&method, &params, &mut state)
                }
                ServerMessage::Request { .. } => Vec::new(),
            })
            .collect()
    }

    fn waits(method: &str, params: Value, state: &mut State) -> Vec<Option<String>> {
        on_notification(method, &params, state)
            .into_iter()
            .filter_map(|i| match i {
                Incoming::Waiting(q) => Some(q),
                _ => None,
            })
            .collect()
    }

    /// A completed `plan` item is the plan the agent put to the person (the
    /// completed item is authoritative, its text is whole); a started one,
    /// an empty one and another item say nothing.
    #[test]
    fn a_completed_plan_item_is_a_proposed_plan() {
        let mut state = State::default();
        let mut item = |done: &str, text: &str| {
            on_notification(
                done,
                &json!({"threadId": THREAD, "item": {"type": "plan", "id": "p1", "text": text}}),
                &mut state,
            )
        };
        let plans = |v: Vec<Incoming>| -> Vec<String> {
            v.into_iter()
                .filter_map(|i| match i {
                    Incoming::Plan(t) => Some(t),
                    _ => None,
                })
                .collect()
        };
        assert!(plans(item("item/started", "1. a")).is_empty());
        assert!(plans(item("item/completed", "  ")).is_empty());
        assert_eq!(plans(item("item/completed", "1. a\n2. b")), ["1. a\n2. b"]);
    }

    const CHILD: &str = "01a14232-aaaa-7d40-8466-1ec0e927ef99";

    /// The recorded collab thread (`tests/fixtures/codex/collab.jsonl`, Codex
    /// 0.162.1): a parent that spawns one sub-agent to count a file's lines
    /// and waits for it. The child's own notifications are in it.
    #[test]
    fn a_recorded_codex_collab_agent_is_attributed() {
        const PARENT: &str = "01a1479e-91e3-7f41-aaa4-d869ae37eb7d";
        const KID: &str = "01a1479e-ac05-71f0-8c17-da181af4fcc9";
        let mut state = State {
            thread: Some(PARENT.into()),
            ..State::default()
        };
        let mut seen: Vec<String> = Vec::new();
        for line in include_str!("../../tests/fixtures/codex/collab.jsonl")
            .lines()
            .skip(1)
        {
            let v: Value = serde_json::from_str(line).unwrap();
            let message = ServerMessage::Notification {
                method: v["method"].as_str().unwrap().into(),
                params: v["params"].clone(),
            };
            for incoming in on_message(&message, &mut state) {
                seen.push(match incoming {
                    Incoming::Subagent {
                        change: SubagentChange::Started,
                        id,
                        kind,
                    } => {
                        format!("started {id} {}", kind.unwrap_or_default())
                    }
                    Incoming::Subagent {
                        change: SubagentChange::Finished(_),
                        id,
                        ..
                    } => {
                        format!("finished {id}")
                    }
                    Incoming::ChildCall { child, call } => {
                        format!(
                            "child {} {:?} {}",
                            child.id,
                            call.kind,
                            call.title.contains("wc -l")
                        )
                    }
                    Incoming::Waiting(_) => "waiting".into(),
                    Incoming::PromptDone(_) => "turn done".into(),
                    Incoming::Update(u) => match *u {
                        ChatUpdate::ToolCall(t) if t.kind == ToolKind::Subagent => {
                            format!("collab {}", t.name.unwrap_or_default())
                        }
                        _ => continue,
                    },
                    _ => continue,
                });
            }
        }
        assert_eq!(
            seen,
            [
                format!("started {KID} /root/count_lines"),
                "collab wait".to_string(),
                format!("child {KID} Execute true"),
                // The child's finished turn is not the parent's; the parent's
                // own turn end is the only one.
                format!("finished {KID}"),
                "collab wait".to_string(),
                "turn done".to_string(),
            ],
            "start, the child's command, finish, in order; the wait is the parent's own call"
        );
    }

    /// The recorded inject (`tests/fixtures/codex/inject.jsonl`, the 0.163
    /// daemon): a developer item appended to an idle thread starts nothing
    /// and says nothing; the person's prompt is the next turn; the
    /// compaction after it is a turn of its own. The connection may inject
    /// only while no turn runs.
    #[test]
    fn a_recorded_inject_starts_no_turn_and_waits_for_none() {
        let mut lines = include_str!("../../tests/fixtures/codex/inject.jsonl").lines();
        let _header = lines.next();
        let first: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let thread = first["params"]["thread"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let mut state = State {
            thread: Some(thread),
            ..State::default()
        };
        let mut log: Vec<String> = Vec::new();
        for line in lines {
            let v: Value = serde_json::from_str(line).unwrap();
            let Some(method) = v.get("method").and_then(Value::as_str) else {
                log.push(
                    v.get("sent")
                        .and_then(Value::as_str)
                        .unwrap_or("answered")
                        .to_string(),
                );
                continue;
            };
            let message = ServerMessage::Notification {
                method: method.into(),
                params: v["params"].clone(),
            };
            let incoming = on_message(&message, &mut state);
            let kind = if incoming.iter().any(|i| matches!(i, Incoming::Compacted)) {
                "compacted"
            } else if incoming
                .iter()
                .any(|i| matches!(i, Incoming::PromptDone(_)))
            {
                "done"
            } else {
                continue;
            };
            log.push(format!("{kind} running={}", state.running));
        }
        assert_eq!(
            log,
            [
                "thread/inject_items",
                "answered",
                "done running=false",
                "thread/compact/start",
                "compacted running=true",
                "done running=false",
            ],
            "the turn ends with nothing running; the compaction is inside a running turn"
        );
        assert!(!state.running);
    }

    /// Between `turn/started` and `turn/completed` the connection must not
    /// inject, and a daemon that doesn't know the method is remembered.
    #[test]
    fn nothing_is_injected_into_a_running_turn() {
        let mut state = State {
            thread: Some("t".into()),
            ..State::default()
        };
        assert!(!state.running);
        on_notification("turn/started", &json!({"threadId": "t"}), &mut state);
        assert!(state.running);
        on_notification(
            "turn/completed",
            &json!({"threadId": "t", "turn": {"status": "completed"}}),
            &mut state,
        );
        assert!(!state.running);
        assert!(inject_unsupported(
            "`thread/inject_items`: Invalid request: unknown variant `thread/inject_items`"
        ));
        assert!(!inject_unsupported("`thread/inject_items`: no such thread"));
    }

    fn note(method: &str, params: Value) -> ServerMessage {
        ServerMessage::Notification {
            method: method.into(),
            params,
        }
    }

    /// What a message makes of a parent thread with a child agent: the
    /// collab call is the parent's own tool call; the child's start and end
    /// bracket a subagent; the child's finished tool calls are the
    /// subagent's, whose own talk, tokens and waits are not the parent's.
    /// Observed only: nothing is sent to the child.
    #[test]
    fn a_collab_agent_is_a_subagent_of_the_thread() {
        let mut state = State {
            thread: Some(THREAD.into()),
            ..State::default()
        };
        let mut all = |m: ServerMessage| on_message(&m, &mut state);
        let subagents = |v: &[Incoming]| -> Vec<(String, bool)> {
            v.iter()
                .filter_map(|i| match i {
                    Incoming::Subagent { change, id, .. } => {
                        Some((id.clone(), matches!(change, SubagentChange::Finished(_))))
                    }
                    _ => None,
                })
                .collect()
        };
        // The parent's spawn call: its own tool call, in oxplow's words a
        // subagent call, naming the child.
        let spawn = json!({"threadId": THREAD, "item": {
            "type": "collabAgentToolCall", "id": "c1", "tool": "spawnAgent",
            "status": "completed", "senderThreadId": THREAD,
            "receiverThreadIds": [CHILD], "prompt": "count the lines",
            "agentsStates": {CHILD: {"status": "pendingInit"}}}});
        let got = all(note("item/completed", spawn));
        let call = got
            .iter()
            .find_map(|i| match i {
                Incoming::Update(u) => match &**u {
                    ChatUpdate::ToolCall(t) => Some(t.clone()),
                    _ => None,
                },
                _ => None,
            })
            .expect("the collab call is a tool call");
        assert_eq!(call.kind, ToolKind::Subagent);
        assert_eq!(call.name.as_deref(), Some("spawnAgent"));
        assert_eq!(
            subagents(&got),
            [(CHILD.to_string(), false)],
            "pending: started"
        );

        // The same child again says nothing new.
        let started = json!({"threadId": THREAD, "item": {
            "type": "subAgentActivity", "id": "a1", "kind": "started",
            "agentThreadId": CHILD, "agentPath": "/root/worker"}});
        assert!(subagents(&all(note("item/completed", started))).is_empty());

        // The child's finished command is the subagent's; its talk,
        // tokens and waits are not the parent's.
        let ran = json!({"threadId": CHILD, "item": {
            "type": "commandExecution", "id": "k1", "status": "completed",
            "command": "wc -l a.txt", "cwd": "/w", "exitCode": 0,
            "aggregatedOutput": "3 a.txt", "commandActions": []}});
        let got = all(note("item/completed", ran));
        let [Incoming::ChildCall { child, call }] = got.as_slice() else {
            panic!("not one child call: {} messages", got.len())
        };
        assert_eq!(child.id, CHILD);
        assert_eq!(call.kind, ToolKind::Execute);
        for quiet in [
            note(
                "item/completed",
                json!({"threadId": CHILD, "item": {"type": "agentMessage", "id": "m", "text": "hi"}}),
            ),
            note(
                "thread/status/changed",
                json!({"threadId": CHILD, "status": {"type": "active", "activeFlags": ["waitingOnApproval"]}}),
            ),
            note(
                "turn/completed",
                json!({"threadId": CHILD, "turn": {"status": "completed"}}),
            ),
        ] {
            assert!(all(quiet).is_empty());
        }
        // A thread that is nobody's child stays out.
        assert!(all(note("item/completed", json!({"threadId": "other", "item": {"type": "commandExecution", "id": "z", "status": "completed", "command": "ls"}}))).is_empty());

        // It finishes, saying what it said last.
        let done = json!({"threadId": THREAD, "item": {
            "type": "collabAgentToolCall", "id": "c2", "tool": "wait",
            "status": "completed", "senderThreadId": THREAD,
            "receiverThreadIds": [CHILD],
            "agentsStates": {CHILD: {"status": "completed", "message": "3 lines"}}}});
        let got = all(note("item/completed", done));
        let finished: Vec<_> = got
            .iter()
            .filter_map(|i| match i {
                Incoming::Subagent {
                    change: SubagentChange::Finished(said),
                    ..
                } => said.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(finished, ["3 lines"]);
        // And only once.
        let activity = json!({"threadId": THREAD, "item": {
            "type": "subAgentActivity", "id": "a2", "kind": "completed",
            "agentThreadId": CHILD, "agentPath": "/root/worker"}});
        assert!(subagents(&all(note("item/completed", activity))).is_empty());
    }

    fn status(flags: &[&str]) -> Value {
        json!({"threadId": THREAD, "status": {"type": "active", "activeFlags": flags}})
    }

    /// The thread waiting on an approval or an answer is the agent waiting
    /// on the person: the status says so, with what it asked when the
    /// request was seen, and goes back once the flags clear, the request is
    /// resolved or the thread idles. Only observed: nothing answers.
    #[test]
    fn a_thread_waiting_on_the_person_is_reported_with_what_it_asked() {
        let mut state = State {
            thread: Some(THREAD.into()),
            ..State::default()
        };
        // An approval, with the command the request named.
        note_request(
            "item/commandExecution/requestApproval",
            &json!({"threadId": THREAD, "command": "rm -rf build"}),
            &mut state,
        );
        assert_eq!(
            waits(
                "thread/status/changed",
                status(&["waitingOnApproval"]),
                &mut state
            ),
            [Some("Approve in Codex: rm -rf build".to_string())]
        );
        // The same state again says nothing new.
        assert!(waits(
            "thread/status/changed",
            status(&["waitingOnApproval"]),
            &mut state
        )
        .is_empty());
        assert_eq!(
            waits("thread/status/changed", status(&[]), &mut state),
            [None]
        );
        // A question: its text.
        note_request(
            "item/tool/requestUserInput",
            &json!({"threadId": THREAD, "questions": [{"question": "Which branch?"}]}),
            &mut state,
        );
        assert_eq!(
            waits(
                "thread/status/changed",
                status(&["waitingOnUserInput"]),
                &mut state
            ),
            [Some("Which branch?".to_string())]
        );
        // Resolved, whichever way it was answered.
        assert_eq!(
            waits(
                "serverRequest/resolved",
                json!({"threadId": THREAD, "requestId": 3}),
                &mut state
            ),
            [None]
        );
        // A wait whose request this client never saw still says so.
        assert_eq!(
            waits(
                "thread/status/changed",
                status(&["waitingOnApproval"]),
                &mut state
            ),
            [Some("Waiting on an approval in Codex".to_string())]
        );
        assert_eq!(
            waits(
                "thread/status/changed",
                json!({"threadId": THREAD, "status": {"type": "idle"}}),
                &mut state
            ),
            [None]
        );
        // Not waiting, nothing to clear.
        assert!(waits(
            "thread/status/changed",
            json!({"threadId": THREAD, "status": {"type": "idle"}}),
            &mut state
        )
        .is_empty());
    }

    #[test]
    fn a_recorded_thread_maps_to_turns_tools_names_and_tokens() {
        let got = mapped();
        let users: Vec<&str> = got
            .iter()
            .filter_map(|i| match i {
                Incoming::Update(u) => match u.as_ref() {
                    ChatUpdate::UserChunk(t) => Some(t.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            users,
            [
                "say hi",
                "run a shell command to append the current time to notes.txt",
                "now directly edit it to append a joke"
            ]
        );
        let tools: Vec<&ToolCall> = got
            .iter()
            .filter_map(|i| match i {
                Incoming::Update(u) => match u.as_ref() {
                    ChatUpdate::ToolCall(t) => Some(t),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let shell = tools
            .iter()
            .rfind(|t| t.title == "date >> notes.txt")
            .unwrap();
        assert_eq!(
            (shell.kind, shell.status),
            (ToolKind::Execute, ToolStatus::Completed)
        );
        assert_eq!(shell.raw_output, Some(json!({"exit_code": 0})));
        let read = tools.iter().rfind(|t| t.title == "cat notes.txt").unwrap();
        assert_eq!(read.kind, ToolKind::Read);
        assert_eq!(read.locations, ["/w/notes.txt"]);
        let edit = tools.iter().rfind(|t| t.kind == ToolKind::Edit).unwrap();
        assert_eq!(edit.status, ToolStatus::Completed);
        assert_eq!(edit.locations, ["/w/notes.txt"]);
        assert!(edit.text[0].contains("+Why do programmers prefer dark mode?"));
        // Streamed text isn't repeated when its item completes.
        let answer: String = got
            .iter()
            .filter_map(|i| match i {
                Incoming::Update(u) => match u.as_ref() {
                    ChatUpdate::AgentChunk(t) => Some(t.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(answer.matches("Appended a joke to `notes.txt`.").count(), 1);
        let ends: Vec<&TurnEnd> = got
            .iter()
            .filter_map(|i| match i {
                Incoming::PromptDone(Ok(end)) => Some(end),
                _ => None,
            })
            .collect();
        assert_eq!(ends.len(), 3);
        assert!(ends.iter().all(|e| e.stop_reason == "completed"));
        let tokens = ends[2].usage.as_ref().unwrap();
        assert!(tokens.output > 0 && tokens.cache_read > 0, "{tokens:?}");
        assert!(got
            .iter()
            .any(|i| matches!(i, Incoming::Named(n) if n == "Say hi")));
    }

    /// A finished item that carries no status (a web search, an image
    /// view) is done, not still running — a resumed thread's included.
    #[test]
    fn a_finished_item_without_a_status_is_completed() {
        let search = json!({"type": "webSearch", "id": "w1", "query": "codex"});
        let done = item_updates(&search, true, &mut HashSet::new());
        let started = item_updates(&search, false, &mut HashSet::new());
        let status = |u: &[ChatUpdate]| match &u[0] {
            ChatUpdate::ToolCall(t) => t.status,
            other => panic!("{other:?}"),
        };
        assert_eq!(status(&done), ToolStatus::Completed);
        assert_eq!(status(&started), ToolStatus::InProgress);
    }

    /// The thread opens on a model and effort; a settings change moves
    /// either; the turn's end carries what they are then.
    #[test]
    fn a_turn_ends_on_the_model_and_effort_the_thread_has() {
        let end_of = |state: &mut State| match on_notification(
            "turn/completed",
            &json!({"threadId": "t", "turn": {"id": "u", "status": "completed"}}),
            state,
        )
        .remove(0)
        {
            Incoming::PromptDone(Ok(end)) => end.run,
            _ => panic!("not a turn end"),
        };
        let run = |model: &str, effort: &str| {
            Some(TurnRun {
                model: Some(model.into()),
                effort: Some(effort.into()),
                ..TurnRun::default()
            })
        };
        let mut state = State::default();
        // Nothing said yet.
        assert_eq!(end_of(&mut state), None);

        state.run =
            thread_run(&json!({"thread": {"model": "gpt-5.6", "reasoningEffort": "medium"}}));
        assert_eq!(end_of(&mut state), run("gpt-5.6", "medium"));

        // A null effort (the model has none) leaves it unset.
        state.run = thread_run(&json!({"thread": {"model": "gpt-5.6", "reasoningEffort": null}}));
        assert_eq!(
            end_of(&mut state),
            Some(TurnRun {
                model: Some("gpt-5.6".into()),
                effort: None,
                ..TurnRun::default()
            })
        );

        let settings = |s: Value| json!({"threadId": "t", "threadSettings": s});
        assert!(on_notification(
            "thread/settings/updated",
            &settings(json!({"model": "gpt-5.7", "effort": "high"})),
            &mut state
        )
        .is_empty());
        assert_eq!(end_of(&mut state), run("gpt-5.7", "high"));
        // A change that names only one keeps the other.
        on_notification(
            "thread/settings/updated",
            &settings(json!({"effort": "low"})),
            &mut state,
        );
        assert_eq!(end_of(&mut state), run("gpt-5.7", "low"));
    }

    /// The last request's tokens are the context's fill and the model's
    /// window its size; the turn's end carries both as they were last said.
    #[test]
    fn a_turn_ends_on_the_context_the_thread_has_filled() {
        let mut state = State::default();
        let usage = json!({"threadId": "t", "turnId": "u", "tokenUsage": {
            "total": {"inputTokens": 900, "cachedInputTokens": 0, "outputTokens": 5, "totalTokens": 905},
            "last": {"inputTokens": 300, "cachedInputTokens": 200, "outputTokens": 5, "totalTokens": 52_000},
            "modelContextWindow": 200_000}});
        on_notification("thread/tokenUsage/updated", &usage, &mut state);
        let ended = on_notification(
            "turn/completed",
            &json!({"threadId": "t", "turn": {"id": "u", "status": "completed"}}),
            &mut state,
        )
        .remove(0);
        let Incoming::PromptDone(Ok(end)) = ended else {
            panic!("not a turn end");
        };
        let run = end.run.unwrap();
        assert_eq!(run.context_tokens, Some(52_000));
        assert_eq!(run.context_window, Some(200_000));
    }

    /// A finished `contextCompaction` item is the thread's context being
    /// summarized; one that only started is not yet.
    #[test]
    fn a_finished_compaction_item_is_reported() {
        let mut state = State::default();
        let item = json!({"threadId": "t", "turnId": "u", "item": {"type": "contextCompaction", "id": "c"}});
        assert!(on_notification("item/started", &item, &mut state).is_empty());
        let done = on_notification("item/completed", &item, &mut state);
        assert!(matches!(done.as_slice(), [Incoming::Compacted]));
    }

    #[test]
    fn a_failed_turn_is_an_error_and_mcp_calls_read_as_mcp() {
        let mut state = State::default();
        let done = on_notification(
            "turn/completed",
            &json!({"threadId": "t", "turn": {"id": "u", "status": "failed", "error": {"message": "quota"}}}),
            &mut state,
        );
        assert!(matches!(&done[0], Incoming::PromptDone(Err(e)) if e == "quota"));
        let call = tool_call(
            &json!({"type": "mcpToolCall", "id": "m1", "server": "oxplow", "tool": "oxplow_read", "status": "completed", "arguments": {"uri": "oxplow:oxplow/wiki?slug=x"}, "result": {"content": []}}),
            "m1".into(),
            true,
        )
        .unwrap();
        let used = super::super::mapping::tool_use(&call).unwrap();
        assert_eq!(used.kind, oxplow_domain::agent::tool::ToolKind::Mcp);
        assert_eq!(used.name, "mcp__oxplow__oxplow_read");
    }
}
