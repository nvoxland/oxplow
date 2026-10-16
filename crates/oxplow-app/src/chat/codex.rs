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
use oxplow_harnesses::codex_app_server::{AppServer, ServerMessage};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use super::acp::{cap, cap_value};
use super::conn::{Connection, Incoming, Opened, TurnEnd, TurnTokens};
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
    /// `thread/fork`), sandboxed to what the thread may do. A new thread is
    /// persisted at once, so Codex's TUI can resume it.
    async fn open(&self, _cwd: &Path, writable: bool) -> Result<Opened, String> {
        let mut params = self.open.params.clone();
        params["sandbox"] = json!(if writable {
            "workspace-write"
        } else {
            "read-only"
        });
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
        self.state.lock().thread = Some(thread.clone());
        let _ = self.opened.send(true);
        Ok(Opened {
            session_id: thread,
            preamble: None,
        })
    }

    /// The person prompts in Codex's TUI; nothing here sends a turn.
    fn prompt(&self, _prompt: HumanPrompt, _writable: bool) -> Result<(), String> {
        Err("a Codex session is prompted in Codex's own TUI".into())
    }

    /// The TUI stops its own turns.
    fn cancel(&self) {}

    /// The thread's sandbox for the turns from now on — the TUI's included
    /// — to what the thread may do: the write guard, as the thread is
    /// promoted or demoted.
    fn set_writable(&self, writable: bool) {
        let sandbox = if writable {
            json!({"type": "workspaceWrite"})
        } else {
            json!({"type": "readOnly"})
        };
        let (server, params) = (
            self.server.clone(),
            json!({"threadId": self.thread(), "sandboxPolicy": sandbox}),
        );
        tokio::spawn(async move {
            if let Err(e) = server.request("thread/settings/update", params).await {
                tracing::warn!(error = %e, "codex: setting the thread's sandbox failed");
            }
        });
    }

    async fn closed(&self) {
        self.server.closed().await;
    }
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
        let ours = {
            let s = state.lock();
            s.thread.is_some() && message.thread_id() == s.thread.as_deref()
        };
        if !ours {
            continue;
        }
        // A request (an approval, a question) is the TUI's to answer.
        let ServerMessage::Notification { method, params } = message else {
            continue;
        };
        for incoming in on_notification(&method, &params, &mut state.lock()) {
            if tx.send(incoming).is_err() {
                return;
            }
        }
    }
}

fn on_notification(method: &str, p: &Value, state: &mut State) -> Vec<Incoming> {
    let update = |u: ChatUpdate| vec![Incoming::Update(Box::new(u))];
    match method {
        "item/started" | "item/completed" => p
            .get("item")
            .map(|item| item_updates(item, method == "item/completed", &mut state.streamed))
            .unwrap_or_default()
            .into_iter()
            .map(|u| Incoming::Update(Box::new(u)))
            .collect(),
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
        "turn/completed" => {
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
                }),
            };
            vec![Incoming::PromptDone(done)]
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
        | "contextCompaction" | "enteredReviewMode" | "exitedReviewMode" | "sleep" => Vec::new(),
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
            &json!({"type": "mcpToolCall", "id": "m1", "server": "oxplow", "tool": "oxplow_read", "status": "completed", "arguments": {"ref": "wiki:x"}, "result": {"content": []}}),
            "m1".into(),
            true,
        )
        .unwrap();
        let used = super::super::mapping::tool_use(&call).unwrap();
        assert_eq!(used.kind, oxplow_domain::agent::tool::ToolKind::Mcp);
        assert_eq!(used.name, "mcp__oxplow__oxplow_read");
    }
}
