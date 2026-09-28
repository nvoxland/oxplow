//! A scripted fake ACP agent (tsk337). It speaks raw newline-delimited
//! JSON-RPC, deliberately NOT the SDK oxplow's client uses, so the client's
//! tests exercise real wire JSON instead of SDK-to-SDK round trips.
//!
//! The fake does what the prompt tells it: every text line of the form
//! `fake:<command> <args>` is one step, in order. Other lines (including an
//! attached oxplow context block) are ignored.
//!
//! | step | what the fake does |
//! |---|---|
//! | `say <text>` | an agent message chunk |
//! | `think <text>` | a thought chunk |
//! | `edit <path>` | an edit tool call, then a permission request; reports `permission: <option id or cancelled>` |
//! | `edit-anyway <path>` | like `edit`, but completes whatever the answer |
//! | `bypass <path>` | an edit tool call that completes without asking |
//! | `fswrite <path> <content>` | an edit tool call that writes through `fs/write_text_file`; reports `fs ok` or `fs error: <message>` |
//! | `fsread <path>` | `fs/read_text_file`; reports `read: <content>` |
//! | `bash <command>` | an execute tool call completing with exit code 0 |
//! | `plan <a>\|<b>` | a plan |
//! | `usage <used> <size>` | a `usage_update` |
//! | `tokens <in> <out>` | per-turn usage on the prompt response |
//! | `wait` | blocks until `session/cancel`, then stops `cancelled` |
//! | `crash` | stops serving at once (the binary then exits 1) |
//!
//! Every `session/update` is kept per session, so `session/load` replays
//! the history. [`FakeState`] records what the client sent for asserts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};

/// What the fake advertises.
#[derive(Debug, Clone)]
pub struct FakeOptions {
    pub load_session: bool,
    pub mcp_http: bool,
    /// Saved after every turn, so a killed fake's sessions still load.
    pub state_file: Option<std::path::PathBuf>,
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self {
            load_session: true,
            mcp_http: true,
            state_file: None,
        }
    }
}

/// Shared between connections (so a second connection can `session/load`
/// what the first recorded) and read by tests.
#[derive(Debug, Default)]
pub struct FakeState {
    /// session id → every update sent in it, for replay.
    pub history: HashMap<String, Vec<Value>>,
    /// The params of every `session/prompt` received.
    pub prompts: Vec<Value>,
    /// The params of every `session/new` received.
    pub new_sessions: Vec<Value>,
    /// The params of every `session/load` received.
    pub loads: Vec<Value>,
    next_session: u64,
    /// Tool-call ids are unique per connection lifetime, like a real agent's.
    next_tool: u64,
}

pub type Shared = Arc<Mutex<FakeState>>;

/// How a connection ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The client closed its end.
    Eof,
    /// A `crash` step ran.
    Crashed,
}

struct Conn<R, W> {
    lines: Lines<BufReader<R>>,
    out: W,
    next_id: u64,
    cancelled: bool,
    /// A `crash` step ran or the client went away mid-turn.
    ended: Option<Ended>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Conn<R, W> {
    async fn send(&mut self, v: Value) -> std::io::Result<()> {
        let mut line = v.to_string();
        line.push('\n');
        self.out.write_all(line.as_bytes()).await?;
        self.out.flush().await
    }

    async fn read(&mut self) -> std::io::Result<Option<Value>> {
        loop {
            let Some(line) = self.lines.next_line().await? else {
                return Ok(None);
            };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                return Ok(Some(v));
            }
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> std::io::Result<()> {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    /// Send a request to the client and wait for its response, noting a
    /// `session/cancel` that arrives meanwhile. `Err(error object)` for an
    /// error response; `None` when the client went away.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> std::io::Result<Option<Result<Value, Value>>> {
        self.next_id += 1;
        let id = format!("fake-{}", self.next_id);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        loop {
            let Some(msg) = self.read().await? else {
                self.ended = Some(Ended::Eof);
                return Ok(None);
            };
            if msg.get("method").and_then(Value::as_str) == Some("session/cancel") {
                self.cancelled = true;
                continue;
            }
            if msg.get("method").is_none() && msg.get("id").and_then(Value::as_str) == Some(&id) {
                return Ok(Some(match msg.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                }));
            }
            if let (Some(req_id), Some(_)) = (msg.get("id"), msg.get("method")) {
                // The client should not send requests mid-turn.
                let req_id = req_id.clone();
                self.send(json!({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32600, "message": "busy"}}))
                    .await?;
            }
        }
    }
}

/// Serve one client connection until it closes or a `crash` step runs.
pub async fn serve<R, W>(
    read: R,
    write: W,
    state: Shared,
    opts: FakeOptions,
) -> std::io::Result<Ended>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut conn = Conn {
        lines: BufReader::new(read).lines(),
        out: write,
        next_id: 0,
        cancelled: false,
        ended: None,
    };
    loop {
        let Some(msg) = conn.read().await? else {
            return Ok(Ended::Eof);
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = msg.get("id").cloned() else {
            // Notifications outside a turn (a late cancel) change nothing.
            continue;
        };
        let result = match method {
            "initialize" => json!({
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": opts.load_session,
                    "mcpCapabilities": {"http": opts.mcp_http, "sse": false},
                    "promptCapabilities": {}
                },
                "authMethods": []
            }),
            "session/new" => {
                let mut s = lock(&state);
                s.next_session += 1;
                let sid = format!("fake-session-{}", s.next_session);
                s.new_sessions.push(params);
                s.history.entry(sid.clone()).or_default();
                json!({"sessionId": sid})
            }
            "session/load" => {
                let sid = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let replay = {
                    let mut s = lock(&state);
                    s.loads.push(params.clone());
                    s.history.get(&sid).cloned()
                };
                match replay {
                    Some(updates) => {
                        for u in updates {
                            conn.notify("session/update", json!({"sessionId": sid, "update": u}))
                                .await?;
                        }
                        json!({})
                    }
                    None => {
                        conn.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32002, "message": "unknown session"}}))
                            .await?;
                        continue;
                    }
                }
            }
            "session/prompt" => {
                lock(&state).prompts.push(params.clone());
                let out = run_prompt(&mut conn, &state, &params).await?;
                if let Some(path) = &opts.state_file {
                    lock(&state).save(path)?;
                }
                if let Some(ended) = conn.ended.take() {
                    return Ok(ended);
                }
                out
            }
            _ => {
                conn.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}))
                    .await?;
                continue;
            }
        };
        conn.send(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await?;
    }
}

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, FakeState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

async fn run_prompt<R, W>(
    conn: &mut Conn<R, W>,
    state: &Shared,
    params: &Value,
) -> std::io::Result<Value>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    conn.cancelled = false;
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let texts: Vec<String> = params
        .get("prompt")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut turn = Turn {
        sid: sid.clone(),
        state: state.clone(),
    };
    for t in &texts {
        turn.update(
            conn,
            json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": t}}),
        )
        .await?;
    }

    let mut usage = None;
    let steps: Vec<(String, String)> = texts
        .iter()
        .flat_map(|t| t.lines())
        .filter_map(|l| l.trim().strip_prefix("fake:"))
        .map(|l| match l.split_once(' ') {
            Some((c, a)) => (c.to_string(), a.to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect();
    for (cmd, arg) in steps {
        if conn.cancelled {
            break;
        }
        match cmd.as_str() {
            "say" => turn.say(conn, &arg).await?,
            "think" => {
                turn.update(conn, json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": arg}}))
                    .await?
            }
            "edit" | "edit-anyway" => {
                let id = turn.tool_call(conn, "edit", &arg, json!({"file_path": arg}), "pending").await?;
                let answer = conn
                    .request(
                        "session/request_permission",
                        json!({
                            "sessionId": sid,
                            "toolCall": {"toolCallId": id, "title": format!("Edit {arg}"), "kind": "edit", "locations": [{"path": arg}]},
                            "options": [
                                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                                {"optionId": "always", "name": "Always allow", "kind": "allow_always"},
                                {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
                            ]
                        }),
                    )
                    .await?;
                let Some(answer) = answer else { break };
                let outcome = answer
                    .ok()
                    .and_then(|r| r.get("outcome").cloned())
                    .unwrap_or(Value::Null);
                let chosen = match outcome.get("outcome").and_then(Value::as_str) {
                    Some("selected") => outcome
                        .get("optionId")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string(),
                    _ => "cancelled".to_string(),
                };
                // `edit-anyway` ignores the answer, like an adapter that
                // writes without honouring a reject.
                let status = if cmd == "edit-anyway" || chosen == "allow" || chosen == "always" {
                    "completed"
                } else {
                    "failed"
                };
                turn.tool_status(conn, &id, status, None).await?;
                turn.say(conn, &format!("permission: {chosen}")).await?;
            }
            "bypass" => {
                turn.tool_call(conn, "edit", &arg, json!({"file_path": arg}), "completed")
                    .await?;
            }
            "fswrite" => {
                let (path, content) = arg.split_once(' ').unwrap_or((arg.as_str(), ""));
                let id = turn
                    .tool_call(conn, "edit", path, json!({"file_path": path}), "in_progress")
                    .await?;
                let r = conn
                    .request(
                        "fs/write_text_file",
                        json!({"sessionId": sid, "path": path, "content": content}),
                    )
                    .await?;
                let Some(r) = r else { break };
                match r {
                    Ok(_) => {
                        turn.tool_status(conn, &id, "completed", None).await?;
                        turn.say(conn, "fs ok").await?;
                    }
                    Err(e) => {
                        turn.tool_status(conn, &id, "failed", None).await?;
                        let msg = e.get("message").and_then(Value::as_str).unwrap_or("");
                        turn.say(conn, &format!("fs error: {msg}")).await?;
                    }
                }
            }
            "fsread" => {
                let r = conn
                    .request("fs/read_text_file", json!({"sessionId": sid, "path": arg}))
                    .await?;
                let Some(r) = r else { break };
                let text = match r {
                    Ok(v) => v
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    Err(e) => format!(
                        "error {}",
                        e.get("message").and_then(Value::as_str).unwrap_or("")
                    ),
                };
                turn.say(conn, &format!("read: {text}")).await?;
            }
            "bash" => {
                let id = turn
                    .tool_call(conn, "execute", &arg, json!({"command": arg}), "in_progress")
                    .await?;
                turn.tool_status(conn, &id, "completed", Some(json!({"exit_code": 0})))
                    .await?;
            }
            "plan" => {
                let entries: Vec<Value> = arg
                    .split('|')
                    .map(|e| json!({"content": e.trim(), "priority": "medium", "status": "pending"}))
                    .collect();
                turn.update(conn, json!({"sessionUpdate": "plan", "entries": entries}))
                    .await?;
            }
            "usage" => {
                let mut n = arg.split_whitespace().map(|x| x.parse::<u64>().unwrap_or(0));
                let (used, size) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
                turn.update(conn, json!({"sessionUpdate": "usage_update", "used": used, "size": size}))
                    .await?;
            }
            "tokens" => {
                let mut n = arg.split_whitespace().map(|x| x.parse::<u64>().unwrap_or(0));
                let (i, o) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
                usage = Some(json!({"totalTokens": i + o, "inputTokens": i, "outputTokens": o}));
            }
            "wait" => {
                while !conn.cancelled {
                    match conn.read().await? {
                        None => {
                            conn.ended = Some(Ended::Eof);
                            return Ok(Value::Null);
                        }
                        Some(m) if m.get("method").and_then(Value::as_str) == Some("session/cancel") => {
                            conn.cancelled = true;
                        }
                        Some(_) => {}
                    }
                }
            }
            "crash" => {
                conn.ended = Some(Ended::Crashed);
                return Ok(Value::Null);
            }
            _ => {}
        }
        if conn.ended.is_some() {
            return Ok(Value::Null);
        }
    }
    let mut out = json!({"stopReason": if conn.cancelled { "cancelled" } else { "end_turn" }});
    if let Some(u) = usage {
        out["usage"] = u;
    }
    Ok(out)
}

struct Turn {
    sid: String,
    state: Shared,
}

impl Turn {
    async fn update<R, W>(&mut self, conn: &mut Conn<R, W>, update: Value) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        lock(&self.state)
            .history
            .entry(self.sid.clone())
            .or_default()
            .push(update.clone());
        conn.notify(
            "session/update",
            json!({"sessionId": self.sid, "update": update}),
        )
        .await
    }

    async fn say<R, W>(&mut self, conn: &mut Conn<R, W>, text: &str) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        self.update(
            conn,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}),
        )
        .await
    }

    async fn tool_call<R, W>(
        &mut self,
        conn: &mut Conn<R, W>,
        kind: &str,
        target: &str,
        raw_input: Value,
        status: &str,
    ) -> std::io::Result<String>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let n = {
            let mut s = lock(&self.state);
            s.next_tool += 1;
            s.next_tool
        };
        let id = format!("{}-t{n}", self.sid);
        let mut call = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": format!("{kind} {target}"),
            "kind": kind,
            "status": status,
            "rawInput": raw_input,
        });
        if kind == "edit" {
            call["locations"] = json!([{"path": target}]);
        }
        self.update(conn, call).await?;
        Ok(id)
    }

    async fn tool_status<R, W>(
        &mut self,
        conn: &mut Conn<R, W>,
        id: &str,
        status: &str,
        raw_output: Option<Value>,
    ) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut u =
            json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "status": status});
        if let Some(o) = raw_output {
            u["rawOutput"] = o;
        }
        self.update(conn, u).await
    }
}

impl FakeState {
    /// The history and session counter from `path` (the binary keeps
    /// them there so a restarted fake can `session/load`).
    pub fn load(path: &std::path::Path) -> Self {
        let v: Value = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let history = v
            .get("history")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_array().cloned().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            history,
            next_session: v.get("nextSession").and_then(Value::as_u64).unwrap_or(0),
            ..Default::default()
        }
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        // `newSessions` is for inspection (what the client sent), not reloaded.
        let v = json!({
            "history": self.history,
            "nextSession": self.next_session,
            "newSessions": self.new_sessions,
        });
        std::fs::write(path, v.to_string())
    }
}
