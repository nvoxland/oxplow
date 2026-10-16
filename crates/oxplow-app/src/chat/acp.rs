//! The only module that names `agent-client-protocol` schema types. It
//! converts them to and from `model.rs`, so the pinned SDK (`=2.2.0`, a
//! young 2.x) can change without touching the rest of `chat/`. It also owns
//! the ACP connection ([`run`], [`AcpConnection`]).

use agent_client_protocol::schema::v1 as sdk;

use crate::chat::conn::{
    Connection, Incoming, Opened, PermissionReply, ReadReply, TurnEnd, TurnTokens, WriteReply,
};
use crate::chat::model::{
    ChatUpdate, ContextUsage, PermissionAnswer, PermissionAsk, PermissionKind, PermissionOption,
    PlanEntry, PlanStatus, ToolCall, ToolCallPatch, ToolDiff, ToolKind, ToolStatus,
};

/// Text kept per content block; a runaway command output is cut here.
pub const MAX_TEXT: usize = 64 * 1024;

pub fn update(u: sdk::SessionUpdate) -> ChatUpdate {
    use sdk::SessionUpdate as U;
    match u {
        U::UserMessageChunk(c) => ChatUpdate::UserChunk(block_text(&c.content)),
        U::AgentMessageChunk(c) => ChatUpdate::AgentChunk(block_text(&c.content)),
        U::AgentThoughtChunk(c) => ChatUpdate::ThoughtChunk(block_text(&c.content)),
        U::ToolCall(t) => ChatUpdate::ToolCall(tool_call(t)),
        U::ToolCallUpdate(u) => ChatUpdate::ToolCallUpdate(tool_patch(u)),
        U::Plan(p) => ChatUpdate::Plan(
            p.entries
                .into_iter()
                .map(|e| PlanEntry {
                    content: e.content,
                    status: match e.status {
                        sdk::PlanEntryStatus::Completed => PlanStatus::Completed,
                        sdk::PlanEntryStatus::InProgress => PlanStatus::InProgress,
                        _ => PlanStatus::Pending,
                    },
                })
                .collect(),
        ),
        U::UsageUpdate(u) => ChatUpdate::Usage(ContextUsage {
            used: u.used,
            size: u.size,
            cost_amount: u.cost.as_ref().map(|c| c.amount),
            cost_currency: u.cost.map(|c| c.currency),
        }),
        _ => ChatUpdate::Ignored,
    }
}

pub fn tool_call(t: sdk::ToolCall) -> ToolCall {
    let (diffs, text) = contents(t.content);
    ToolCall {
        id: t.tool_call_id.0.to_string(),
        title: t.title,
        name: t.name,
        kind: kind(t.kind),
        status: status(t.status),
        locations: t.locations.iter().map(location).collect(),
        raw_input: t.raw_input,
        raw_output: t.raw_output.map(cap_value),
        diffs,
        text,
    }
}

pub fn tool_patch(u: sdk::ToolCallUpdate) -> ToolCallPatch {
    let f = u.fields;
    let (diffs, text) = match f.content {
        Some(c) => {
            let (d, t) = contents(c);
            (Some(d), Some(t))
        }
        None => (None, None),
    };
    ToolCallPatch {
        id: u.tool_call_id.0.to_string(),
        title: f.title,
        name: f.name,
        kind: f.kind.map(kind),
        status: f.status.map(status),
        locations: f.locations.map(|l| l.iter().map(location).collect()),
        raw_input: f.raw_input,
        raw_output: f.raw_output.map(cap_value),
        diffs,
        text,
    }
}

pub fn permission_ask(r: sdk::RequestPermissionRequest) -> PermissionAsk {
    PermissionAsk {
        tool: tool_patch(r.tool_call),
        options: r
            .options
            .into_iter()
            .map(|o| PermissionOption {
                id: o.option_id.0.to_string(),
                name: o.name,
                kind: match o.kind {
                    sdk::PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
                    sdk::PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
                    sdk::PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
                    _ => PermissionKind::RejectOnce,
                },
            })
            .collect(),
    }
}

pub fn permission_response(a: &PermissionAnswer) -> sdk::RequestPermissionResponse {
    let outcome = match a {
        PermissionAnswer::Selected { option_id } => sdk::RequestPermissionOutcome::Selected(
            sdk::SelectedPermissionOutcome::new(option_id.clone()),
        ),
        PermissionAnswer::Cancelled => sdk::RequestPermissionOutcome::Cancelled,
    };
    sdk::RequestPermissionResponse::new(outcome)
}

fn kind(k: sdk::ToolKind) -> ToolKind {
    use sdk::ToolKind as K;
    match k {
        K::Read => ToolKind::Read,
        K::Edit => ToolKind::Edit,
        K::Delete => ToolKind::Delete,
        K::Move => ToolKind::Move,
        K::Search => ToolKind::Search,
        K::Execute => ToolKind::Execute,
        K::Think => ToolKind::Think,
        K::Fetch => ToolKind::Fetch,
        K::SwitchMode => ToolKind::SwitchMode,
        _ => ToolKind::Other,
    }
}

fn status(s: sdk::ToolCallStatus) -> ToolStatus {
    use sdk::ToolCallStatus as S;
    match s {
        S::InProgress => ToolStatus::InProgress,
        S::Completed => ToolStatus::Completed,
        S::Failed => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn location(l: &sdk::ToolCallLocation) -> String {
    l.path.to_string_lossy().into_owned()
}

fn contents(c: Vec<sdk::ToolCallContent>) -> (Vec<ToolDiff>, Vec<String>) {
    let mut diffs = Vec::new();
    let mut text = Vec::new();
    for item in c {
        match item {
            sdk::ToolCallContent::Diff(d) => diffs.push(ToolDiff {
                path: d.path.to_string_lossy().into_owned(),
                old_text: d.old_text,
                new_text: d.new_text,
            }),
            sdk::ToolCallContent::Content(c) => text.push(block_text(&c.content)),
            // No terminal capability is advertised, so none should arrive.
            _ => {}
        }
    }
    (diffs, text)
}

/// Text for a content block; non-text blocks become a short placeholder.
fn block_text(b: &sdk::ContentBlock) -> String {
    match b {
        sdk::ContentBlock::Text(t) => cap(&t.text),
        other => {
            let v = serde_json::to_value(other).unwrap_or_default();
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("content");
            match v
                .get("uri")
                .or_else(|| v.pointer("/resource/uri"))
                .and_then(|u| u.as_str())
            {
                Some(uri) => format!("[{ty}: {uri}]"),
                None => format!("[{ty}]"),
            }
        }
    }
}

pub(crate) fn cap(s: &str) -> String {
    if s.len() <= MAX_TEXT {
        return s.to_string();
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated]", &s[..end])
}

pub(crate) fn cap_value(v: serde_json::Value) -> serde_json::Value {
    match &v {
        serde_json::Value::String(s) if s.len() > MAX_TEXT => serde_json::Value::String(cap(s)),
        serde_json::Value::String(_) => v,
        _ if v.to_string().len() > MAX_TEXT => serde_json::json!({ "truncated": true }),
        _ => v,
    }
}

// ---------------------------------------------------------------------
// The connection: the SDK's client role, fenced behind oxplow types.
// ---------------------------------------------------------------------

/// What `initialize` told us about the agent.
#[derive(Debug, Clone, PartialEq)]
struct AgentInfo {
    load_session: bool,
    mcp_http: bool,
}

/// oxplow's MCP server as an HTTP MCP entry for `session/new|load`.
#[derive(Debug, Clone, PartialEq)]
pub struct McpHttp {
    pub name: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// How an ACP session opens.
#[derive(Debug, Clone, Default)]
pub struct AcpOpen {
    /// The `acpAgents` name, for errors.
    pub agent: String,
    pub mcp: Vec<McpHttp>,
    /// The agent session's last ACP session, loaded instead of starting
    /// fresh when the agent supports `session/load`.
    pub resume_session_id: Option<String>,
    pub system_prompt: Option<String>,
    /// Pass the system prompt as `_meta.systemPrompt.append` on
    /// `session/new` (the Claude adapter) instead of ahead of the first
    /// prompt.
    pub system_prompt_via_meta: bool,
}

type Tx = tokio::sync::mpsc::UnboundedSender<Incoming>;

/// The client side of one ACP agent connection.
pub struct AcpConnection {
    conn: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>,
    tx: Tx,
    open: AcpOpen,
    session_id: std::sync::OnceLock<String>,
}

fn err_text(e: agent_client_protocol::Error) -> String {
    e.message.clone()
}

fn from_json<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("building request: {e}"))
}

fn mcp_json(servers: &[McpHttp]) -> serde_json::Value {
    serde_json::Value::Array(
        servers
            .iter()
            .map(|s| {
                serde_json::json!({
                    "type": "http",
                    "name": s.name,
                    "url": s.url,
                    "headers": s.headers.iter().map(|(n, v)| serde_json::json!({"name": n, "value": v})).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

#[async_trait::async_trait]
impl Connection for AcpConnection {
    /// `initialize`, then `session/load` of the saved session when the
    /// agent supports it (its replay arrives as updates first), else
    /// `session/new`.
    /// Its writes are gated as it asks (`fs/write_text_file`, permission
    /// requests), so `writable` changes nothing here.
    async fn open(&self, cwd: &std::path::Path, _writable: bool) -> Result<Opened, String> {
        let info = self.initialize().await?;
        if !self.open.mcp.is_empty() && !info.mcp_http {
            return Err(format!(
                "ACP agent '{}' can't use HTTP MCP servers, and oxplow's tools reach ACP agents over HTTP MCP",
                self.open.agent
            ));
        }
        let saved = self
            .open
            .resume_session_id
            .clone()
            .filter(|s| !s.is_empty());
        if let (Some(id), true) = (saved, info.load_session) {
            match self.load_session(&id, cwd).await {
                Ok(()) => {
                    let _ = self.session_id.set(id.clone());
                    return Ok(Opened {
                        session_id: id,
                        preamble: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "acp: session/load failed; starting a new session")
                }
            }
        }
        let meta = match (&self.open.system_prompt, self.open.system_prompt_via_meta) {
            (Some(sp), true) => Some(serde_json::json!({"systemPrompt": {"append": sp}})),
            _ => None,
        };
        let id = self.new_session(cwd, meta).await?;
        let _ = self.session_id.set(id.clone());
        Ok(Opened {
            session_id: id,
            preamble: (!self.open.system_prompt_via_meta)
                .then(|| self.open.system_prompt.clone())
                .flatten(),
        })
    }

    fn prompt(
        &self,
        p: crate::chat::human_prompt::HumanPrompt,
        _writable: bool,
    ) -> Result<(), String> {
        let blocks: Vec<serde_json::Value> = p
            .blocks()
            .iter()
            .map(|t| serde_json::json!({"type": "text", "text": t}))
            .collect();
        let req: sdk::PromptRequest = from_json(serde_json::json!({
            "sessionId": self.session_id(),
            "prompt": blocks,
        }))?;
        let tx = self.tx.clone();
        self.conn
            .send_request(req)
            .on_receiving_result(async move |r| {
                let done = r.map_err(err_text).map(|r| TurnEnd {
                    stop_reason: serde_json::to_value(r.stop_reason)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    usage: r.usage.map(|u| TurnTokens {
                        input: u.input_tokens,
                        output: u.output_tokens,
                        cache_read: u.cached_read_tokens.unwrap_or(0),
                        cache_write: u.cached_write_tokens.unwrap_or(0),
                    }),
                });
                let _ = tx.send(Incoming::PromptDone(done));
                Ok(())
            })
            .map_err(err_text)
    }

    fn cancel(&self) {
        if let Ok(n) = from_json::<sdk::CancelNotification>(
            serde_json::json!({"sessionId": self.session_id()}),
        ) {
            let _ = self.conn.send_notification(n);
        }
    }

    /// The agent's output reached EOF (it exited).
    async fn closed(&self) {
        self.conn.incoming_closed().await;
    }
}

impl AcpConnection {
    fn session_id(&self) -> &str {
        self.session_id
            .get()
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// `initialize`: fs read/write offered, no terminal.
    async fn initialize(&self) -> Result<AgentInfo, String> {
        let req: sdk::InitializeRequest = from_json(serde_json::json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {"readTextFile": true, "writeTextFile": true},
                "terminal": false
            },
            "clientInfo": {"name": "oxplow", "version": env!("CARGO_PKG_VERSION")}
        }))?;
        let r = self
            .conn
            .send_request(req)
            .block_task()
            .await
            .map_err(err_text)?;
        Ok(AgentInfo {
            load_session: r.agent_capabilities.load_session,
            mcp_http: r.agent_capabilities.mcp_capabilities.http,
        })
    }

    async fn new_session(
        &self,
        cwd: &std::path::Path,
        meta: Option<serde_json::Value>,
    ) -> Result<String, String> {
        let mut v = serde_json::json!({"cwd": cwd, "mcpServers": mcp_json(&self.open.mcp)});
        if let Some(m) = meta {
            v["_meta"] = m;
        }
        let req: sdk::NewSessionRequest = from_json(v)?;
        let r = self
            .conn
            .send_request(req)
            .block_task()
            .await
            .map_err(err_text)?;
        Ok(r.session_id.0.to_string())
    }

    /// `session/load`. The agent replays the history as updates BEFORE this
    /// returns; they arrive on the incoming channel first.
    async fn load_session(&self, session_id: &str, cwd: &std::path::Path) -> Result<(), String> {
        let req: sdk::LoadSessionRequest = from_json(serde_json::json!({
            "sessionId": session_id,
            "cwd": cwd,
            "mcpServers": mcp_json(&self.open.mcp),
        }))?;
        // Ordered: the response is queued behind the replayed updates.
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        self.conn
            .send_request(req)
            .on_receiving_result(async move |r| {
                let _ = done_tx.send(r.map(|_| ()).map_err(err_text));
                Ok(())
            })
            .map_err(err_text)?;
        done_rx
            .await
            .map_err(|_| "connection closed during session/load".to_string())?
    }
}

fn agent_error(message: String) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32000, message)
}

/// Run a client connection over `write`/`read` (the agent's stdin and
/// stdout), opening as `open` says. `main` gets the connection and the
/// incoming channel; the connection lives until `main` returns.
pub async fn run<W, R, F, Fut>(write: W, read: R, open: AcpOpen, main: F) -> Result<(), String>
where
    W: tokio::io::AsyncWrite + Send + 'static,
    R: tokio::io::AsyncRead + Send + 'static,
    F: FnOnce(std::sync::Arc<AcpConnection>, tokio::sync::mpsc::UnboundedReceiver<Incoming>) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
    let transport = agent_client_protocol::ByteStreams::new(write.compat_write(), read.compat());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Incoming>();
    let (t1, t2, t3, t4) = (tx.clone(), tx.clone(), tx.clone(), tx.clone());
    agent_client_protocol::Client
        .builder()
        .name("oxplow")
        .on_receive_notification(
            async move |n: sdk::SessionNotification, _cx| {
                let _ = t1.send(Incoming::Update(Box::new(update(n.update))));
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |r: sdk::RequestPermissionRequest, responder, _cx| {
                let _ = t2.send(Incoming::Permission {
                    ask: Box::new(permission_ask(r)),
                    reply: PermissionReply::new(move |a| {
                        let _ = responder.respond(permission_response(a));
                    }),
                });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |r: sdk::WriteTextFileRequest, responder, _cx| {
                let _ = t3.send(Incoming::WriteFile {
                    path: r.path,
                    content: r.content,
                    reply: WriteReply::new(move |done| {
                        let _ = match done {
                            Ok(()) => responder.respond(sdk::WriteTextFileResponse::new()),
                            Err(reason) => responder.respond_with_error(agent_error(reason)),
                        };
                    }),
                });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |r: sdk::ReadTextFileRequest, responder, _cx| {
                let _ = t4.send(Incoming::ReadFile {
                    path: r.path,
                    line: r.line,
                    limit: r.limit,
                    reply: ReadReply::new(move |read| {
                        let _ = match read {
                            Ok(content) => {
                                responder.respond(sdk::ReadTextFileResponse::new(content))
                            }
                            Err(message) => responder.respond_with_error(agent_error(message)),
                        };
                    }),
                });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |conn| {
            let connection = std::sync::Arc::new(AcpConnection {
                conn,
                tx,
                open,
                session_id: std::sync::OnceLock::new(),
            });
            main(connection, rx).await;
            Ok(())
        })
        .await
        .map_err(err_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upd(v: serde_json::Value) -> ChatUpdate {
        update(serde_json::from_value(v).unwrap())
    }

    #[test]
    fn chunks_become_text() {
        assert_eq!(
            upd(
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}})
            ),
            ChatUpdate::AgentChunk("hi".into())
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hm"}})
            ),
            ChatUpdate::ThoughtChunk("hm".into())
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "user_message_chunk", "content": {"type": "resource_link", "uri": "file:///a.rs", "name": "a.rs"}})
            ),
            ChatUpdate::UserChunk("[resource_link: file:///a.rs]".into())
        );
    }

    #[test]
    fn tool_call_carries_diffs_locations_and_input() {
        let ChatUpdate::ToolCall(t) = upd(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "c1",
            "title": "Edit a.rs",
            "kind": "edit",
            "status": "pending",
            "locations": [{"path": "/w/a.rs", "line": 3}],
            "rawInput": {"file_path": "/w/a.rs"},
            "content": [
                {"type": "diff", "path": "/w/a.rs", "oldText": "x", "newText": "y"},
                {"type": "content", "content": {"type": "text", "text": "note"}}
            ]
        })) else {
            panic!("not a tool call")
        };
        assert_eq!(t.id, "c1");
        assert_eq!(t.kind, ToolKind::Edit);
        assert_eq!(t.status, ToolStatus::Pending);
        assert_eq!(t.locations, vec!["/w/a.rs".to_string()]);
        assert_eq!(t.raw_input, Some(json!({"file_path": "/w/a.rs"})));
        assert_eq!(
            t.diffs,
            vec![ToolDiff {
                path: "/w/a.rs".into(),
                old_text: Some("x".into()),
                new_text: "y".into()
            }]
        );
        assert_eq!(t.text, vec!["note".to_string()]);
    }

    #[test]
    fn tool_call_update_is_a_sparse_patch() {
        let ChatUpdate::ToolCallUpdate(p) = upd(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "c1",
            "status": "completed",
            "rawOutput": {"exit_code": 0}
        })) else {
            panic!("not an update")
        };
        assert_eq!(p.id, "c1");
        assert_eq!(p.status, Some(ToolStatus::Completed));
        assert_eq!(p.raw_output, Some(json!({"exit_code": 0})));
        assert_eq!(p.title, None);
        assert_eq!(p.diffs, None);
    }

    #[test]
    fn plan_and_usage() {
        assert_eq!(
            upd(json!({"sessionUpdate": "plan", "entries": [
                {"content": "a", "priority": "high", "status": "completed"},
                {"content": "b", "priority": "low", "status": "pending"}
            ]})),
            ChatUpdate::Plan(vec![
                PlanEntry {
                    content: "a".into(),
                    status: PlanStatus::Completed
                },
                PlanEntry {
                    content: "b".into(),
                    status: PlanStatus::Pending
                },
            ])
        );
        assert_eq!(
            upd(
                json!({"sessionUpdate": "usage_update", "used": 10, "size": 100, "cost": {"amount": 0.5, "currency": "USD"}})
            ),
            ChatUpdate::Usage(ContextUsage {
                used: 10,
                size: 100,
                cost_amount: Some(0.5),
                cost_currency: Some("USD".into())
            })
        );
        assert_eq!(
            upd(json!({"sessionUpdate": "current_mode_update", "currentModeId": "x"})),
            ChatUpdate::Ignored
        );
    }

    #[test]
    fn long_text_is_capped() {
        let big = "é".repeat(MAX_TEXT);
        let ChatUpdate::AgentChunk(t) = upd(
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": big}}),
        ) else {
            panic!()
        };
        assert!(t.len() < MAX_TEXT + 32);
        assert!(t.ends_with("[truncated]"));
    }

    #[test]
    fn permission_round_trip() {
        let ask = permission_ask(
            serde_json::from_value(json!({
                "sessionId": "s1",
                "toolCall": {"toolCallId": "c1", "title": "Write b.rs", "kind": "edit"},
                "options": [
                    {"optionId": "a", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "r", "name": "Reject", "kind": "reject_once"}
                ]
            }))
            .unwrap(),
        );
        assert_eq!(ask.tool.id, "c1");
        assert_eq!(ask.tool.kind, Some(ToolKind::Edit));
        assert_eq!(ask.options[1].kind, PermissionKind::RejectOnce);

        let sel = serde_json::to_value(permission_response(&PermissionAnswer::Selected {
            option_id: "r".into(),
        }))
        .unwrap();
        assert_eq!(
            sel,
            json!({"outcome": {"outcome": "selected", "optionId": "r"}})
        );
        let cancel =
            serde_json::to_value(permission_response(&PermissionAnswer::Cancelled)).unwrap();
        assert_eq!(cancel, json!({"outcome": {"outcome": "cancelled"}}));
    }
}
