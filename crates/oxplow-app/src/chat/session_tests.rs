//! The session actor against a scripted in-process agent (`FakeConn`, a
//! [`Connection`]) and a recording host in place of `Services`; then a Codex
//! thread on a scripted app-server daemon.

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{DenyLayer, PolicyDecision};
use parking_lot::Mutex;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Notify};

use super::conn::{
    Connection, Incoming, Opened, PermissionReply, ReadReply, TurnEnd, TurnTokens, WriteReply,
};
use super::host::ChatHost;
use super::human_prompt::HumanPrompt;
use super::manager::ChatManager;
use super::model::{
    ChatUpdate, PermissionAnswer, PermissionAsk, PermissionKind, PermissionOption, ToolCall,
    ToolCallPatch, ToolKind, ToolStatus,
};
use super::session::{ChatError, ChatEvent, ChatEventBody, ChatStatus, SessionSpec};
use super::transcript::ItemBody;
use oxplow_domain::agent::tool::{ToolKind as Kind, ToolUse};

#[derive(Default)]
struct Host {
    /// Deny every worktree write with this reason.
    deny_writes: Option<String>,
    context: Option<String>,
    /// The signal a change to the context bumps.
    changes: Option<tokio::sync::watch::Sender<u64>>,
    log: Mutex<Vec<String>>,
}

impl Host {
    fn log(&self) -> Vec<String> {
        self.log.lock().clone()
    }
    fn count(&self, prefix: &str) -> usize {
        self.log().iter().filter(|l| l.starts_with(prefix)).count()
    }
}

#[async_trait]
impl ChatHost for Host {
    async fn check_tool(
        &self,
        _t: &ThreadId,
        _s: &str,
        tool: &ToolUse,
        _p: &serde_json::Value,
    ) -> PolicyDecision {
        self.log
            .lock()
            .push(format!("check {} {}", tool.name, tool.paths.join(",")));
        match (&self.deny_writes, tool.kind) {
            (Some(r), Kind::Edit) => PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason: r.clone(),
            },
            _ => PolicyDecision::Allow,
        }
    }
    async fn session_started(&self, _t: &ThreadId, s: &str) {
        self.log.lock().push(format!("started {s}"));
    }
    async fn prompt_context(
        &self,
        _t: &ThreadId,
        _s: &str,
    ) -> Option<crate::agent_context::PendingContext> {
        self.context
            .clone()
            .map(crate::agent_context::PendingContext::plain)
    }
    async fn context_delivered(&self, context: crate::agent_context::PendingContext) {
        self.log.lock().push(format!("delivered {}", context.text));
    }
    fn context_changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        self.changes.as_ref().map(|s| s.subscribe())
    }
    async fn turn_started(&self, _t: &ThreadId, _s: &str, prompt: &str) {
        self.log.lock().push(format!("turn_started {prompt}"));
    }
    async fn tool_finished(&self, _t: &ThreadId, _s: &str, tool: &ToolUse, _c: &serde_json::Value) {
        self.log.lock().push(format!("tool {}", tool.kind.as_str()));
    }
    async fn turn_ended(
        &self,
        _t: &ThreadId,
        _s: &str,
        answer: Option<&str>,
        tokens: Option<&TurnTokens>,
        _run: Option<&oxplow_domain::hook::TurnRun>,
    ) {
        self.log.lock().push(format!(
            "turn_ended {}{}",
            tokens
                .map(|t| format!("{}/{}", t.input, t.output))
                .unwrap_or_default(),
            answer.map(|a| format!(" answer={a}")).unwrap_or_default()
        ));
    }
    async fn named(&self, _t: &ThreadId, _s: &str, title: &str) {
        self.log.lock().push(format!("named {title}"));
    }
    async fn subagent(
        &self,
        _t: &ThreadId,
        _s: &str,
        change: &crate::chat::conn::SubagentChange,
        id: &str,
        _kind: Option<&str>,
    ) {
        let what = match change {
            crate::chat::conn::SubagentChange::Started => "started",
            crate::chat::conn::SubagentChange::Finished(_) => "finished",
        };
        self.log.lock().push(format!("subagent {what} {id}"));
    }
    async fn plan_proposed(&self, _t: &ThreadId, _s: &str, text: &str) {
        self.log.lock().push(format!("plan {text}"));
    }
    async fn compacted(&self, _t: &ThreadId, _s: &str) {
        self.log.lock().push("compacted".into());
    }
    async fn awaiting_user(&self, _t: &ThreadId, q: Option<String>) {
        self.log.lock().push(format!("awaiting {}", q.is_some()));
    }
    async fn interrupted(&self, _t: &ThreadId) {
        self.log.lock().push("interrupted".into());
    }
    fn activity(&self, _t: &ThreadId) {}
}

fn thread() -> ThreadId {
    ThreadId::new(7)
}

fn session() -> oxplow_domain::AgentSessionId {
    oxplow_domain::AgentSessionId::new(9)
}

fn spec(cwd: &std::path::Path) -> SessionSpec {
    SessionSpec {
        session_id: session(),
        thread_id: thread(),
        agent: "fake".into(),
        cwd: cwd.to_path_buf(),
    }
}

// ---------------------------------------------------------------------
// The scripted agent.
//
// It does what the prompt tells it: every text line of the form
// `fake:<command> <args>` is one step, in order; other lines (an attached
// context block) are ignored.
//
// | step | what the agent does |
// |---|---|
// | `say <text>` | an agent message chunk |
// | `think <text>` | a thought chunk |
// | `edit <path>` | an edit tool call, then a permission request; reports `permission: <option id or cancelled>` |
// | `edit-anyway <path>` | like `edit`, but completes whatever the answer |
// | `bypass <path>` | an edit tool call that completes without asking |
// | `fswrite <path> <content>` | an edit tool call that asks oxplow to write the file; reports `fs ok` or `fs error: <message>` |
// | `fsread <path>` | asks oxplow to read the file; reports `read: <content>` |
// | `bash <command>` | an execute tool call completing with exit code 0 |
// | `tokens <in> <out>` | per-turn counts on the turn's end |
// | `wait` | blocks until cancelled, then ends the turn `cancelled` |
// | `crash` | the agent goes away at once |
// ---------------------------------------------------------------------

/// What the agent keeps between connections (a reopen resumes its
/// history) and what tests read back.
#[derive(Default)]
struct FakeState {
    /// session id → every update sent in it, replayed on resume.
    history: HashMap<String, Vec<ChatUpdate>>,
    /// The blocks of every prompt received.
    prompts: Vec<Vec<String>>,
    /// How each open went: `new` or `load <id>`.
    opens: Vec<String>,
    next_session: u64,
    next_tool: u64,
}

type Shared = Arc<Mutex<FakeState>>;

#[derive(Default, Clone)]
struct FakeOptions {
    /// Resume this session (replaying its history) instead of a new one.
    resume: Option<String>,
    /// The system prompt the agent has no other way to take.
    preamble: Option<String>,
    /// Refuse to open, with this message.
    refuse: Option<String>,
}

struct Inner {
    state: Shared,
    tx: mpsc::UnboundedSender<Incoming>,
    sid: Mutex<String>,
    cancelled: AtomicBool,
    wake: Notify,
    ended: watch::Sender<bool>,
}

struct FakeConn {
    inner: Arc<Inner>,
    opts: FakeOptions,
}

fn connect(
    state: &Shared,
    opts: FakeOptions,
) -> (Arc<dyn Connection>, mpsc::UnboundedReceiver<Incoming>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (ended, _) = watch::channel(false);
    let inner = Arc::new(Inner {
        state: state.clone(),
        tx,
        sid: Mutex::new(String::new()),
        cancelled: AtomicBool::new(false),
        wake: Notify::new(),
        ended,
    });
    (Arc::new(FakeConn { inner, opts }), rx)
}

#[async_trait]
impl Connection for FakeConn {
    async fn open(&self, _cwd: &std::path::Path) -> Result<Opened, String> {
        if let Some(why) = &self.opts.refuse {
            return Err(why.clone());
        }
        let i = &self.inner;
        let session_id = {
            let mut s = i.state.lock();
            match &self.opts.resume {
                Some(id) => {
                    let history = s.history.get(id).cloned().ok_or("unknown session")?;
                    s.opens.push(format!("load {id}"));
                    // The replay reaches the incoming channel before open returns.
                    for u in history {
                        let _ = i.tx.send(Incoming::Update(Box::new(u)));
                    }
                    id.clone()
                }
                None => {
                    s.next_session += 1;
                    let id = format!("fake-session-{}", s.next_session);
                    s.history.entry(id.clone()).or_default();
                    s.opens.push("new".into());
                    id
                }
            }
        };
        *i.sid.lock() = session_id.clone();
        Ok(Opened {
            session_id,
            preamble: self.opts.preamble.clone(),
        })
    }

    fn prompt(&self, prompt: HumanPrompt) -> Result<(), String> {
        let inner = self.inner.clone();
        let blocks = prompt.blocks().to_vec();
        inner.state.lock().prompts.push(blocks.clone());
        inner.cancelled.store(false, Ordering::SeqCst);
        tokio::spawn(async move { inner.run_prompt(blocks).await });
        Ok(())
    }

    fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
    }

    async fn closed(&self) {
        let mut rx = self.inner.ended.subscribe();
        let _ = rx.wait_for(|ended| *ended).await;
    }
}

impl Inner {
    fn send(&self, msg: Incoming) {
        let _ = self.tx.send(msg);
    }

    fn update(&self, u: ChatUpdate) {
        let sid = self.sid.lock().clone();
        self.state
            .lock()
            .history
            .entry(sid)
            .or_default()
            .push(u.clone());
        self.send(Incoming::Update(Box::new(u)));
    }

    fn say(&self, text: &str) {
        self.update(ChatUpdate::AgentChunk(text.to_string()));
    }

    fn tool_call(
        &self,
        kind: ToolKind,
        label: &str,
        target: &str,
        raw_input: serde_json::Value,
        status: ToolStatus,
    ) -> String {
        let n = {
            let mut s = self.state.lock();
            s.next_tool += 1;
            s.next_tool
        };
        let id = format!("{}-t{n}", self.sid.lock());
        self.update(ChatUpdate::ToolCall(ToolCall {
            id: id.clone(),
            title: format!("{label} {target}"),
            kind,
            status,
            locations: if kind == ToolKind::Edit {
                vec![target.to_string()]
            } else {
                vec![]
            },
            raw_input: Some(raw_input),
            ..ToolCall::default()
        }));
        id
    }

    fn tool_status(&self, id: &str, status: ToolStatus, raw_output: Option<serde_json::Value>) {
        self.update(ChatUpdate::ToolCallUpdate(ToolCallPatch {
            id: id.to_string(),
            status: Some(status),
            raw_output,
            ..ToolCallPatch::default()
        }));
    }

    async fn run_prompt(&self, blocks: Vec<String>) {
        for b in &blocks {
            self.update(ChatUpdate::UserChunk(b.clone()));
        }
        let mut tokens = None;
        let steps: Vec<(String, String)> = blocks
            .iter()
            .flat_map(|b| b.lines())
            .filter_map(|l| l.trim().strip_prefix("fake:"))
            .map(|l| match l.split_once(' ') {
                Some((c, a)) => (c.to_string(), a.to_string()),
                None => (l.to_string(), String::new()),
            })
            .collect();
        for (cmd, arg) in steps {
            if self.cancelled.load(Ordering::SeqCst) {
                break;
            }
            match cmd.as_str() {
                "say" => self.say(&arg),
                "think" => self.update(ChatUpdate::ThoughtChunk(arg)),
                "edit" | "edit-anyway" => {
                    let id = self.tool_call(
                        ToolKind::Edit,
                        "edit",
                        &arg,
                        serde_json::json!({"file_path": arg}),
                        ToolStatus::Pending,
                    );
                    let (answer_tx, answer_rx) = oneshot::channel();
                    self.send(Incoming::Permission {
                        ask: Box::new(PermissionAsk {
                            tool: ToolCallPatch {
                                id: id.clone(),
                                title: Some(format!("Edit {arg}")),
                                kind: Some(ToolKind::Edit),
                                locations: Some(vec![arg.clone()]),
                                ..ToolCallPatch::default()
                            },
                            options: vec![
                                option("allow", "Allow", PermissionKind::AllowOnce),
                                option("always", "Always allow", PermissionKind::AllowAlways),
                                option("reject", "Reject", PermissionKind::RejectOnce),
                            ],
                        }),
                        reply: PermissionReply::new(move |a| {
                            let _ = answer_tx.send(a.clone());
                        }),
                    });
                    let chosen = match answer_rx.await {
                        Ok(PermissionAnswer::Selected { option_id }) => option_id,
                        _ => "cancelled".to_string(),
                    };
                    // `edit-anyway` ignores the answer, like an adapter that
                    // writes without honouring a reject.
                    let status = if cmd == "edit-anyway" || chosen == "allow" || chosen == "always"
                    {
                        ToolStatus::Completed
                    } else {
                        ToolStatus::Failed
                    };
                    self.tool_status(&id, status, None);
                    self.say(&format!("permission: {chosen}"));
                }
                "bypass" => {
                    self.tool_call(
                        ToolKind::Edit,
                        "edit",
                        &arg,
                        serde_json::json!({"file_path": arg}),
                        ToolStatus::Completed,
                    );
                }
                "fswrite" => {
                    let (path, content) = arg.split_once(' ').unwrap_or((arg.as_str(), ""));
                    let id = self.tool_call(
                        ToolKind::Edit,
                        "edit",
                        path,
                        serde_json::json!({"file_path": path}),
                        ToolStatus::InProgress,
                    );
                    let (done_tx, done_rx) = oneshot::channel();
                    self.send(Incoming::WriteFile {
                        path: path.into(),
                        content: content.to_string(),
                        reply: WriteReply::new(move |r| {
                            let _ = done_tx.send(r);
                        }),
                    });
                    match done_rx.await {
                        Ok(Ok(())) => {
                            self.tool_status(&id, ToolStatus::Completed, None);
                            self.say("fs ok");
                        }
                        Ok(Err(why)) => {
                            self.tool_status(&id, ToolStatus::Failed, None);
                            self.say(&format!("fs error: {why}"));
                        }
                        Err(_) => break,
                    }
                }
                "fsread" => {
                    let (read_tx, read_rx) = oneshot::channel();
                    self.send(Incoming::ReadFile {
                        path: arg.clone().into(),
                        line: None,
                        limit: None,
                        reply: ReadReply::new(move |r| {
                            let _ = read_tx.send(r);
                        }),
                    });
                    let text = match read_rx.await {
                        Ok(Ok(content)) => content,
                        Ok(Err(why)) => format!("error {why}"),
                        Err(_) => break,
                    };
                    self.say(&format!("read: {text}"));
                }
                "bash" => {
                    let id = self.tool_call(
                        ToolKind::Execute,
                        "execute",
                        &arg,
                        serde_json::json!({"command": arg}),
                        ToolStatus::InProgress,
                    );
                    self.tool_status(
                        &id,
                        ToolStatus::Completed,
                        Some(serde_json::json!({"exit_code": 0})),
                    );
                }
                "tokens" => {
                    let mut n = arg
                        .split_whitespace()
                        .map(|x| x.parse::<u64>().unwrap_or(0));
                    tokens = Some(TurnTokens {
                        input: n.next().unwrap_or(0),
                        output: n.next().unwrap_or(0),
                        ..TurnTokens::default()
                    });
                }
                "wait" => {
                    while !self.cancelled.load(Ordering::SeqCst) {
                        self.wake.notified().await;
                    }
                }
                "crash" => {
                    let _ = self.ended.send(true);
                    return;
                }
                _ => {}
            }
        }
        let cancelled = self.cancelled.load(Ordering::SeqCst);
        self.send(Incoming::PromptDone(Ok(TurnEnd {
            stop_reason: if cancelled { "cancelled" } else { "end_turn" }.into(),
            usage: tokens,
            run: None,
        })));
    }
}

fn option(id: &str, name: &str, kind: PermissionKind) -> PermissionOption {
    PermissionOption {
        id: id.into(),
        name: name.into(),
        kind,
    }
}

struct Rig {
    mgr: ChatManager,
    fake: Shared,
    events: broadcast::Receiver<ChatEvent>,
    host: Arc<Host>,
    dir: tempfile::TempDir,
}

/// Open a session on a fresh manager against the agent `fake` holds.
async fn open_with(
    host: Host,
    fake: Shared,
    opts: FakeOptions,
    dir: tempfile::TempDir,
) -> (Rig, Result<(), ChatError>) {
    let mgr = ChatManager::new();
    let host = Arc::new(host);
    let (conn, incoming) = connect(&fake, opts);
    let r = mgr
        .open_connection(host.clone(), spec(dir.path()), conn, incoming)
        .await;
    // After open: the startup `Idle` must not read as a finished turn.
    let events = mgr.subscribe();
    (
        Rig {
            mgr,
            fake,
            events,
            host,
            dir,
        },
        r,
    )
}

async fn open(host: Host) -> Rig {
    let (rig, r) = open_with(
        host,
        Shared::default(),
        FakeOptions::default(),
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    rig
}

impl Rig {
    /// Open the session again on this manager (a Restart).
    async fn reopen(&self, opts: FakeOptions) -> Result<(), ChatError> {
        let (conn, incoming) = connect(&self.fake, opts);
        self.mgr
            .open_connection(self.host.clone(), spec(self.dir.path()), conn, incoming)
            .await
    }

    async fn wait(&mut self, what: &str, pred: impl Fn(&ChatEventBody) -> bool) -> ChatEventBody {
        let fut = async {
            loop {
                let e = self.events.recv().await.unwrap();
                if pred(&e.body) {
                    return e.body;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(5), fut)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    async fn wait_status(&mut self, status: ChatStatus) {
        self.wait(
            &format!("{status:?}"),
            |b| matches!(b, ChatEventBody::Status { status: s } if *s == status),
        )
        .await;
    }

    async fn prompt(&self, text: &str) {
        self.mgr
            .submit_human_prompt(&session(), text.to_string())
            .await
            .unwrap();
    }

    /// Prompt and wait for the turn to end.
    async fn turn(&mut self, text: &str) {
        self.prompt(text).await;
        self.wait_status(ChatStatus::Idle).await;
    }

    fn items(&self) -> Vec<ItemBody> {
        self.mgr
            .transcript(&session(), 0)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.body)
            .collect()
    }

    fn agent_text(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .filter_map(|b| match b {
                ItemBody::Agent { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    fn prompts(&self) -> Vec<Vec<String>> {
        self.fake.lock().prompts.clone()
    }
}

#[tokio::test]
async fn a_prompt_streams_the_reply_and_records_the_turn_once() {
    let mut rig = open(Host::default()).await;
    assert!(rig
        .host
        .log()
        .iter()
        .any(|l| l.starts_with("started fake-session-")));
    rig.turn("hi\nfake:say hello\nfake:bash ls\nfake:tokens 10 5")
        .await;

    let items = rig.items();
    // Exactly what was typed: the agent's live user_message_chunk echo is
    // not appended to it.
    assert!(matches!(
        &items[0],
        ItemBody::User { text, context: None } if text == "hi\nfake:say hello\nfake:bash ls\nfake:tokens 10 5"
    ));
    assert_eq!(rig.agent_text(), vec!["hello".to_string()]);
    assert!(items
        .iter()
        .any(|b| matches!(b, ItemBody::Tool { call } if call.id.ends_with("-t1"))));
    assert_eq!(rig.host.count("turn_started"), 1);
    assert_eq!(rig.host.count("turn_ended"), 1);
    // The turn ends with its counts and the agent's last message as its answer.
    assert!(
        rig.host
            .log()
            .contains(&"turn_ended 10/5 answer=hello".to_string()),
        "{:?}",
        rig.host.log()
    );
    assert!(rig.host.log().contains(&"tool shell".to_string()));
    assert_eq!(rig.prompts().len(), 1);
}

#[tokio::test]
async fn a_denied_edit_is_rejected_without_asking() {
    let mut rig = open(Host {
        deny_writes: Some("another stream's worktree".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:edit /w/a.rs").await;
    let items = rig.items();
    assert!(!items
        .iter()
        .any(|b| matches!(b, ItemBody::Permission { .. })));
    assert!(items.iter().any(
        |b| matches!(b, ItemBody::PolicyDenied { reason, .. } if reason == "another stream's worktree")
    ));
    assert_eq!(rig.agent_text(), vec!["permission: reject".to_string()]);
    assert_eq!(rig.host.count("awaiting"), 0);
}

#[tokio::test]
async fn a_permission_card_waits_for_the_person() {
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:edit /w/a.rs").await;
    let item = match rig
        .wait("card", |b| matches!(b, ChatEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await
    {
        ChatEventBody::Item { item } => item,
        _ => unreachable!(),
    };
    let ItemBody::Permission {
        request_id,
        options,
        ..
    } = item.body
    else {
        unreachable!()
    };
    // No "always allow" for a write.
    let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, vec!["allow", "reject"]);
    assert_eq!(
        rig.mgr.transcript(&session(), 0).unwrap().status,
        ChatStatus::AwaitingPermission
    );
    assert_eq!(
        rig.mgr
            .respond_permission(&session(), request_id.clone(), Some("always".into()))
            .await,
        Err(ChatError::UnknownOption("always".into()))
    );
    rig.mgr
        .respond_permission(&session(), request_id, Some("allow".into()))
        .await
        .unwrap();
    rig.wait_status(ChatStatus::Idle).await;
    assert_eq!(rig.agent_text(), vec!["permission: allow".to_string()]);
    assert!(rig.items().iter().any(|b| matches!(
        b,
        ItemBody::Permission { answer: Some(PermissionAnswer::Selected { option_id }), .. } if option_id == "allow"
    )));
    assert_eq!(
        rig.host
            .log()
            .iter()
            .filter(|l| l.starts_with("awaiting"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["awaiting true", "awaiting false"]
    );
}

#[tokio::test]
async fn an_fs_write_denial_reaches_the_agent_and_an_allowed_one_lands() {
    let mut rig = open(Host {
        deny_writes: Some("start a task first".into()),
        ..Default::default()
    })
    .await;
    let p = rig.dir.path().join("b.txt");
    rig.turn(&format!("fake:fswrite {} hi", p.display())).await;
    assert_eq!(
        rig.agent_text(),
        vec!["fs error: start a task first".to_string()]
    );
    assert!(!p.exists());

    let mut ok = open(Host::default()).await;
    let q = ok.dir.path().join("sub/c.txt");
    ok.turn(&format!("fake:fswrite {} hi", q.display())).await;
    assert_eq!(ok.agent_text(), vec!["fs ok".to_string()]);
    assert_eq!(std::fs::read_to_string(&q).unwrap(), "hi");
    // Written through the gate: not a bypass.
    assert!(!ok
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Bypass { .. })));
}

#[tokio::test]
async fn fs_reads_are_served() {
    let mut rig = open(Host::default()).await;
    let p = rig.dir.path().join("r.txt");
    std::fs::write(&p, "contents").unwrap();
    rig.turn(&format!("fake:fsread {}", p.display())).await;
    assert_eq!(rig.agent_text(), vec!["read: contents".to_string()]);
}

#[tokio::test]
async fn cancel_answers_open_cards() {
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:edit /w/a.rs").await;
    rig.wait("card", |b| matches!(b, ChatEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await;
    rig.mgr.cancel(&session()).unwrap();
    rig.wait_status(ChatStatus::Idle).await;
    assert!(rig.items().iter().any(|b| matches!(
        b,
        ItemBody::Permission {
            answer: Some(PermissionAnswer::Cancelled),
            ..
        }
    )));
    assert_eq!(rig.agent_text(), vec!["permission: cancelled".to_string()]);
}

#[tokio::test]
async fn a_second_prompt_during_a_turn_is_refused_not_queued() {
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:wait").await;
    rig.wait_status(ChatStatus::Running).await;
    assert_eq!(
        rig.mgr
            .submit_human_prompt(&session(), "again".into())
            .await,
        Err(ChatError::TurnInFlight)
    );
    rig.mgr.cancel(&session()).unwrap();
    rig.wait_status(ChatStatus::Idle).await;
    assert_eq!(rig.prompts().len(), 1);
}

/// The host's context (the session context, and the nudges a turn's tool
/// calls left in the store) rides each human prompt.
#[tokio::test]
async fn the_context_rides_each_human_prompt() {
    let mut rig = open(Host {
        context: Some("CTX".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:bash cargo test").await;
    rig.turn("next").await;
    let prompts = rig.prompts();
    assert_eq!(prompts[0], vec!["CTX", "fake:bash cargo test"]);
    assert_eq!(prompts[1], vec!["CTX", "next"]);
}

/// An agent with no other way to take the system prompt gets it ahead of
/// the first prompt, and only that one.
#[tokio::test]
async fn the_system_prompt_rides_ahead_of_the_first_prompt_only() {
    let (mut rig, r) = open_with(
        Host::default(),
        Shared::default(),
        FakeOptions {
            preamble: Some("SYS".into()),
            ..Default::default()
        },
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    rig.turn("a").await;
    rig.turn("b").await;
    let p = rig.prompts();
    assert_eq!(p[0], vec!["SYS", "a"]);
    assert_eq!(p[1], vec!["b"]);
}

#[tokio::test]
async fn an_agent_that_refuses_to_open_is_reported() {
    let (rig, r) = open_with(
        Host::default(),
        Shared::default(),
        FakeOptions {
            refuse: Some("no such model".into()),
            ..Default::default()
        },
        tempfile::tempdir().unwrap(),
    )
    .await;
    let Err(ChatError::Agent(msg)) = r else {
        panic!("expected a refusal")
    };
    assert!(msg.contains("no such model"), "{msg}");
    assert!(!rig.mgr.is_open(&session()));
    assert!(rig
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Error { .. })));
}

#[tokio::test]
async fn a_load_replay_rebuilds_the_transcript_and_records_nothing() {
    let fake: Shared = Shared::default();
    let mut first = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        tempfile::tempdir().unwrap(),
    )
    .await
    .0;
    first.turn("fake:say one\nfake:bash ls").await;
    let sid = first.mgr.transcript(&session(), 0).unwrap();
    assert_eq!(sid.items.len(), 3);
    let session_id = first
        .host
        .log()
        .iter()
        .find_map(|l| l.strip_prefix("started ").map(str::to_string))
        .unwrap();
    first.mgr.close(&session()).unwrap();

    let (second, r) = open_with(
        Host {
            deny_writes: Some("x".into()),
            ..Default::default()
        },
        fake.clone(),
        FakeOptions {
            resume: Some(session_id.clone()),
            ..Default::default()
        },
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    assert_eq!(
        fake.lock().opens,
        vec!["new".to_string(), format!("load {session_id}")],
        "loaded, not new"
    );
    assert_eq!(second.agent_text(), vec!["one".to_string()]);
    assert!(second
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Tool { .. })));
    let log = second.host.log();
    assert_eq!(
        log,
        vec![format!("started {session_id}")],
        "replay recorded: {log:?}"
    );
}

#[tokio::test]
async fn a_write_that_skipped_the_gate_is_flagged() {
    let mut rig = open(Host {
        deny_writes: Some("read-only".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:bypass /w/c.rs").await;
    assert!(rig.items().iter().any(|b| matches!(
        b,
        ItemBody::Bypass { reason, .. } if reason == "read-only"
    )));
}

#[tokio::test]
async fn an_agent_crash_interrupts_the_session() {
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:say bye\nfake:crash").await;
    let closed = rig
        .wait("closed", |b| matches!(b, ChatEventBody::Closed { .. }))
        .await;
    assert!(matches!(closed, ChatEventBody::Closed { reason: Some(_) }));
    assert_eq!(
        rig.mgr.transcript(&session(), 0).unwrap().status,
        ChatStatus::Stopped
    );
    assert!(rig.host.log().contains(&"interrupted".to_string()));
    assert!(!rig.mgr.is_open(&session()));
    assert_eq!(
        rig.mgr.submit_human_prompt(&session(), "x".into()).await,
        Err(ChatError::NotOpen)
    );
}

#[tokio::test]
async fn tool_calls_in_different_turns_stay_distinct() {
    let mut rig = open(Host::default()).await;
    rig.turn("fake:bash one").await;
    rig.turn("fake:bash two").await;
    let tools = rig
        .items()
        .into_iter()
        .filter(|b| matches!(b, ItemBody::Tool { .. }))
        .count();
    assert_eq!(tools, 2);
    assert_eq!(rig.host.count("tool shell"), 2);
}

#[tokio::test]
async fn fs_reads_and_writes_stay_inside_the_sessions_worktree() {
    let mut rig = open(Host::default()).await;
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("x.txt");
    std::fs::write(outside.path().join("secret.txt"), "s3cret").unwrap();
    // `..` out of the worktree is outside too.
    let escape_name = format!(
        "escape-{}.txt",
        rig.dir.path().file_name().unwrap().to_string_lossy()
    );
    let dotdot = rig.dir.path().join("..").join(&escape_name);
    rig.turn(&format!(
        "fake:fswrite {} hi\nfake:fswrite {} hi\nfake:fsread {}",
        target.display(),
        dotdot.display(),
        outside.path().join("secret.txt").display()
    ))
    .await;
    // Consecutive agent chunks merge into one item; read them as one text.
    let text = rig.agent_text().join("|");
    assert_eq!(text.matches("fs error:").count(), 2, "{text}");
    assert!(text.contains("outside this session's worktree"), "{text}");
    assert!(
        text.contains("read: error") && !text.contains("s3cret"),
        "{text}"
    );
    assert!(!target.exists());
    assert!(!dotdot.exists());
}

#[tokio::test]
async fn each_open_is_a_new_generation_and_its_events_say_so() {
    let fake: Shared = Shared::default();
    let (first, r) = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    let g1 = first.mgr.transcript(&session(), 0).unwrap().generation;
    first.mgr.close(&session()).unwrap();
    // Reopening on the same manager (a Restart) starts a new generation.
    let mut events = first.mgr.subscribe();
    tokio::time::sleep(Duration::from_millis(50)).await;
    first.reopen(FakeOptions::default()).await.unwrap();
    let g2 = first.mgr.transcript(&session(), 0).unwrap().generation;
    assert_ne!(g1, g2);
    // Late events from the closed session still carry its generation, so
    // a client can tell them from the new session's.
    let seen = tokio::time::timeout(Duration::from_secs(5), async {
        let mut seen = Vec::new();
        loop {
            let e = events.recv().await.unwrap();
            seen.push(e.generation);
            if e.generation == g2 {
                return seen;
            }
        }
    })
    .await
    .expect("an event from the new session");
    assert!(seen.iter().all(|g| *g == g1 || *g == g2), "{seen:?}");
}

#[tokio::test]
async fn concurrent_opens_start_one_agent() {
    let fake: Shared = Shared::default();
    let mgr = ChatManager::new();
    let host = Arc::new(Host::default());
    let dir = tempfile::tempdir().unwrap();
    let (c1, r1) = connect(&fake, FakeOptions::default());
    let (c2, r2) = connect(&fake, FakeOptions::default());
    let (a, b) = tokio::join!(
        mgr.open_connection(host.clone(), spec(dir.path()), c1, r1),
        mgr.open_connection(host.clone(), spec(dir.path()), c2, r2),
    );
    a.unwrap();
    b.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.lock().opens.len(), 1, "one agent session, not two");
    assert_eq!(host.count("started"), 1);
}

#[tokio::test]
async fn close_is_immediate_and_a_replaced_sessions_teardown_records_nothing() {
    let fake: Shared = Shared::default();
    let (rig, r) = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    rig.mgr.close(&session()).unwrap();
    assert!(!rig.mgr.is_open(&session()), "closed at once");
    // Reopen right away (a Restart) while the old actor may still be
    // shutting down.
    rig.reopen(FakeOptions::default()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let log = rig.host.log();
    let last = log
        .iter()
        .rev()
        .find(|l| l.starts_with("started") || l == &"interrupted")
        .unwrap();
    assert!(
        last.starts_with("started"),
        "the old session's teardown clobbered the new: {log:?}"
    );
}

#[tokio::test]
async fn cancelling_a_card_mid_turn_returns_the_view_to_running() {
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:edit /w/a.rs\nfake:wait").await;
    rig.wait("card", |b| matches!(b, ChatEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await;
    rig.wait_status(ChatStatus::AwaitingPermission).await;
    let request_id = rig
        .items()
        .into_iter()
        .find_map(|b| match b {
            ItemBody::Permission { request_id, .. } => Some(request_id),
            _ => None,
        })
        .unwrap();
    // Answering the card while the turn keeps going: the view says Running.
    rig.mgr
        .respond_permission(&session(), request_id, Some("reject".into()))
        .await
        .unwrap();
    let next = rig
        .wait("status", |b| matches!(b, ChatEventBody::Status { .. }))
        .await;
    assert_eq!(
        next,
        ChatEventBody::Status {
            status: ChatStatus::Running
        }
    );
    rig.mgr.cancel(&session()).unwrap();
    rig.wait_status(ChatStatus::Idle).await;
}

#[tokio::test]
async fn a_write_completed_after_a_reject_is_flagged() {
    // Rejected by the policy.
    let mut rig = open(Host {
        deny_writes: Some("read-only".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:edit-anyway /w/a.rs").await;
    assert!(rig
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Bypass { .. })));

    // Rejected by the person.
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:edit-anyway /w/b.rs").await;
    let ChatEventBody::Item { item } = rig
        .wait("card", |b| matches!(b, ChatEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await
    else {
        unreachable!()
    };
    let ItemBody::Permission { request_id, .. } = item.body else {
        unreachable!()
    };
    rig.mgr
        .respond_permission(&session(), request_id, Some("reject".into()))
        .await
        .unwrap();
    rig.wait_status(ChatStatus::Idle).await;
    let bypass = rig.items().into_iter().find_map(|b| match b {
        ItemBody::Bypass { reason, .. } => Some(reason),
        _ => None,
    });
    assert!(
        bypass.is_some_and(|r| r.contains("rejected")),
        "a reject was ignored"
    );

    // Allowed: no flag.
    let mut rig = open(Host::default()).await;
    rig.prompt("fake:edit /w/c.rs").await;
    let ChatEventBody::Item { item } = rig
        .wait("card", |b| matches!(b, ChatEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await
    else {
        unreachable!()
    };
    let ItemBody::Permission { request_id, .. } = item.body else {
        unreachable!()
    };
    rig.mgr
        .respond_permission(&session(), request_id, Some("allow".into()))
        .await
        .unwrap();
    rig.wait_status(ChatStatus::Idle).await;
    assert!(!rig
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Bypass { .. })));
}

// ---------------------------------------------------------------------
// A Codex thread on a scripted app-server daemon.
// ---------------------------------------------------------------------

mod codex {
    use super::*;
    use crate::chat::codex::CodexOpen;
    use oxplow_harnesses::codex_app_server::fake::FakeAppServer;
    use serde_json::json;

    const THREAD: &str = "thr-x";

    /// A daemon whose `thread/start` opens `thr-x` and whose
    /// `thread/resume` opens it named.
    fn daemon(home: &std::path::Path) -> FakeAppServer {
        daemon_failing(home, 0)
    }

    /// A daemon that refuses its first `fails` context injections.
    fn daemon_failing(home: &std::path::Path, fails: usize) -> FakeAppServer {
        let refusals = std::sync::atomic::AtomicUsize::new(fails);
        FakeAppServer::start(home, move |method, _| match method {
            "thread/start" => Ok(json!({"thread": {"id": THREAD, "name": null}})),
            "thread/resume" => Ok(json!({"thread": {"id": THREAD, "name": "Earlier work"}})),
            "thread/goal/set" | "thread/goal/clear" => Ok(json!({})),
            "thread/name/set" => Ok(json!({})),
            "thread/inject_items" => {
                let left = refusals.load(std::sync::atomic::Ordering::SeqCst);
                if left > 0 {
                    refusals.store(left - 1, std::sync::atomic::Ordering::SeqCst);
                    Err("the daemon is busy".into())
                } else {
                    Ok(json!({}))
                }
            }
            other => Err(format!("no {other}")),
        })
    }

    async fn open_codex(host: Host, method: &str) -> (Rig, FakeAppServer) {
        open_codex_failing(host, method, 0).await
    }

    async fn open_codex_failing(host: Host, method: &str, fails: usize) -> (Rig, FakeAppServer) {
        // Short: a unix socket's path has a length limit.
        let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
        let fake = daemon_failing(dir.path(), fails);
        let mgr = ChatManager::new();
        let host = Arc::new(host);
        let thread = mgr
            .open_codex(
                host.clone(),
                spec(dir.path()),
                CodexOpen {
                    socket: fake.socket.clone(),
                    program: None,
                    method: method.into(),
                    params: json!({"cwd": dir.path()}),
                },
            )
            .await
            .unwrap();
        assert_eq!(thread, THREAD);
        let events = mgr.subscribe();
        (
            Rig {
                mgr,
                fake: Shared::default(),
                events,
                host,
                dir,
            },
            fake,
        )
    }

    fn note(method: &str, params: serde_json::Value) -> serde_json::Value {
        let mut params = params;
        params["threadId"] = json!(THREAD);
        json!({"method": method, "params": params})
    }

    fn methods(fake: &FakeAppServer) -> Vec<String> {
        fake.received()
            .iter()
            .filter_map(|v| v["method"].as_str().map(str::to_string))
            .collect()
    }

    /// A new thread is saved before the TUI attaches (its goal set and
    /// cleared writes its rollout, which `codex resume` needs); a resumed
    /// one already has one.
    #[tokio::test]
    async fn a_new_thread_is_saved_so_the_tui_can_resume_it() {
        let (_rig, fake) = open_codex(Host::default(), "thread/start").await;
        let sent = methods(&fake);
        let start = sent.iter().position(|m| m == "thread/start").unwrap();
        assert_eq!(
            sent[start..start + 3],
            ["thread/start", "thread/goal/set", "thread/goal/clear"]
        );
        let (rig, fake) = open_codex(Host::default(), "thread/resume").await;
        assert!(!methods(&fake).iter().any(|m| m.starts_with("thread/goal")));
        // Named once the session it names is recorded.
        assert_eq!(rig.host.log(), ["started thr-x", "named Earlier work"]);
    }

    /// How many context items the daemon was given so far.
    fn injected(fake: &FakeAppServer) -> usize {
        methods(fake)
            .iter()
            .filter(|m| *m == "thread/inject_items")
            .count()
    }

    /// Wait until `n` items have been injected (and a moment longer, so a
    /// further one would show).
    async fn injected_is(fake: &FakeAppServer, n: usize) {
        for _ in 0..100 {
            if injected(fake) >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(injected(fake), n);
    }

    /// A Codex thread has no prompt to carry context, so the session
    /// appends it to the thread's history: once at open, again when a turn
    /// ends, and after a compaction — never while a turn runs, where the
    /// daemon would read it as more input.
    #[tokio::test]
    async fn context_is_injected_between_turns_never_during_one() {
        let (mut rig, fake) = open_codex(
            Host {
                context: Some("<session-context>x</session-context>".into()),
                ..Host::default()
            },
            "thread/start",
        )
        .await;
        injected_is(&fake, 1).await;
        let sent = fake
            .received()
            .into_iter()
            .find(|v| v["method"] == "thread/inject_items")
            .unwrap();
        assert_eq!(sent["params"]["threadId"], THREAD);
        assert_eq!(sent["params"]["items"][0]["role"], "developer");
        assert_eq!(
            sent["params"]["items"][0]["content"][0]["text"],
            "<session-context>x</session-context>"
        );

        // A turn in the TUI that compacts: nothing goes in until it ends.
        fake.push(note("turn/started", json!({"turn": {"id": "u1"}})));
        fake.push(note("item/completed", json!({"item": {"type": "userMessage", "id": "m1", "content": [{"type": "text", "text": "go"}]}})));
        fake.push(note(
            "item/completed",
            json!({"item": {"type": "contextCompaction", "id": "c1"}}),
        ));
        rig.wait_status(ChatStatus::Running).await;
        injected_is(&fake, 1).await;
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u1", "status": "completed"}}),
        ));
        rig.wait_status(ChatStatus::Idle).await;
        injected_is(&fake, 2).await;
        assert!(rig.host.log().contains(&"compacted".to_string()));
    }

    /// A context the daemon refused is not delivered: nothing is recorded
    /// as sent, so the next chance (the end of a turn) carries it whole.
    #[tokio::test]
    async fn a_refused_inject_loses_nothing() {
        let (mut rig, fake) = open_codex_failing(
            Host {
                context: Some("<handoffs>x</handoffs>".into()),
                ..Host::default()
            },
            "thread/start",
            1,
        )
        .await;
        injected_is(&fake, 1).await;
        assert!(
            !rig.host.log().iter().any(|l| l.starts_with("delivered")),
            "{:?}",
            rig.host.log()
        );
        fake.push(note("turn/started", json!({"turn": {"id": "u1"}})));
        fake.push(note("item/completed", json!({"item": {"type": "userMessage", "id": "m1", "content": [{"type": "text", "text": "go"}]}})));
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u1", "status": "completed"}}),
        ));
        rig.wait_status(ChatStatus::Idle).await;
        injected_is(&fake, 2).await;
        assert_eq!(
            rig.host
                .log()
                .iter()
                .filter(|l| *l == "delivered <handoffs>x</handoffs>")
                .count(),
            1
        );
    }

    /// While no turn runs, a change to what the context says is injected
    /// at once — not after the next turn ends.
    #[tokio::test]
    async fn a_change_while_idle_is_injected_before_the_next_turn() {
        let (changes, _) = tokio::sync::watch::channel(0u64);
        let (rig, fake) = open_codex(
            Host {
                context: Some("<handoffs>y</handoffs>".into()),
                changes: Some(changes.clone()),
                ..Host::default()
            },
            "thread/start",
        )
        .await;
        injected_is(&fake, 1).await;
        changes.send_modify(|n| *n += 1);
        injected_is(&fake, 2).await;
        assert_eq!(rig.host.count("turn_started"), 0, "no turn ran");
    }

    /// A prompt the person types into Codex's TUI is recorded as theirs:
    /// its items stream into the record, a finished command is a tool
    /// call, and `turn/completed` ends the turn with its tokens. Another
    /// thread's notifications don't pass.
    #[tokio::test]
    async fn a_prompt_in_the_tui_is_a_recorded_turn() {
        let (mut rig, fake) = open_codex(Host::default(), "thread/start").await;
        fake.push(note("turn/started", json!({"turn": {"id": "u1"}})));
        fake.push(note("item/completed", json!({"item": {"type": "userMessage", "id": "m1", "content": [{"type": "text", "text": "list the files"}]}})));
        fake.push(json!({"method": "item/agentMessage/delta", "params": {"threadId": "other", "itemId": "z", "delta": "NOT OURS"}}));
        fake.push(note("item/completed", json!({"item": {"type": "commandExecution", "id": "e1", "command": "/bin/zsh -lc ls", "status": "completed", "exitCode": 0, "commandActions": [{"type": "unknown", "command": "ls"}], "aggregatedOutput": "a\n"}})));
        fake.push(note(
            "item/agentMessage/delta",
            json!({"itemId": "a1", "delta": "done"}),
        ));
        fake.push(note(
            "item/completed",
            json!({"item": {"type": "agentMessage", "id": "a1", "text": "done"}}),
        ));
        fake.push(note("thread/tokenUsage/updated", json!({"tokenUsage": {
            "total": {"inputTokens": 100, "cachedInputTokens": 60, "cacheWriteInputTokens": 0, "outputTokens": 7, "totalTokens": 107},
            "last": {"inputTokens": 100, "cachedInputTokens": 60, "cacheWriteInputTokens": 0, "outputTokens": 7, "totalTokens": 107},
            "modelContextWindow": 1000}})));
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u1", "status": "completed"}}),
        ));
        rig.wait_status(ChatStatus::Running).await;
        rig.wait_status(ChatStatus::Idle).await;
        assert_eq!(rig.agent_text(), ["done"]);
        assert_eq!(
            rig.host.log()[1..],
            [
                "turn_started list the files",
                "tool shell",
                "turn_ended 40/7 answer=done"
            ]
        );
        fake.push(note(
            "thread/name/updated",
            json!({"threadName": "TUI work"}),
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !rig.host.log().contains(&"named TUI work".to_string()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    /// The person's rename in oxplow reaches Codex as `thread/name/set` on
    /// the observer's connection, naming the thread; a daemon that refuses
    /// says why, and a session that isn't running can't be renamed.
    #[tokio::test]
    async fn a_native_rename_is_a_thread_name_set() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        rig.mgr
            .rename_native(&session(), "Login fix".into())
            .await
            .unwrap();
        let sent = fake
            .received()
            .into_iter()
            .find(|v| v["method"] == "thread/name/set")
            .expect("the daemon was asked");
        assert_eq!(
            sent["params"],
            json!({"threadId": THREAD, "name": "Login fix"})
        );
        let err = rig
            .mgr
            .rename_native(&oxplow_domain::AgentSessionId::new(404), "x".into())
            .await
            .unwrap_err();
        assert!(matches!(err, ChatError::NotOpen), "{err:?}");
    }

    /// Codex's daemon going away ends the observer; the next open of the
    /// session, against the daemon that came back, resumes the same thread
    /// and observes it again, rather than staying dead or opening another.
    #[tokio::test]
    async fn a_restarted_daemon_is_observed_again_on_the_next_open() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        assert!(rig.mgr.is_open(&session()));
        fake.stop();
        drop(fake);
        let _ = std::fs::remove_file(
            rig.dir
                .path()
                .join("app-server-control/app-server-control.sock"),
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while rig.mgr.is_open(&session()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the observer ends with its daemon");
        let back = daemon(rig.dir.path());
        let thread = rig
            .mgr
            .open_codex(
                rig.host.clone(),
                spec(rig.dir.path()),
                CodexOpen {
                    socket: back.socket.clone(),
                    program: None,
                    method: "thread/resume".into(),
                    params: json!({"threadId": THREAD, "cwd": rig.dir.path()}),
                },
            )
            .await
            .unwrap();
        assert_eq!(thread, THREAD, "the same thread, not another");
        assert!(rig.mgr.is_open(&session()));
        assert!(methods(&back).contains(&"thread/resume".to_string()));
    }

    /// An approval the TUI waits on shows as the agent waiting on the
    /// person, with what it asked, and clears once it is answered there. The
    /// request is read and never answered from here.
    #[tokio::test]
    async fn an_approval_in_the_tui_is_a_wait_that_clears_when_answered() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        fake.push(
            json!({"id": 41, "method": "item/commandExecution/requestApproval", "params": {
            "threadId": THREAD, "turnId": "u1", "itemId": "e1", "command": "rm -rf build"}}),
        );
        fake.push(note(
            "thread/status/changed",
            json!({"status": {"type": "active", "activeFlags": ["waitingOnApproval"]}}),
        ));
        let waited = |rig: &Rig, want: &str| {
            let log = rig.host.log();
            log.iter().any(|l| l == want)
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while !waited(&rig, "awaiting true") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the wait is reported");
        fake.push(note(
            "thread/status/changed",
            json!({"status": {"type": "active", "activeFlags": []}}),
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !waited(&rig, "awaiting false") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("answering clears it");
        // Observed only: nothing was sent back for the request.
        assert!(!fake
            .received()
            .iter()
            .any(|v| v.get("id") == Some(&json!(41)) && v.get("method").is_none()));
    }

    /// A plan the agent completes in the TUI reaches the host once, whole.
    #[tokio::test]
    async fn a_plan_item_in_the_tui_is_handed_to_the_host() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        fake.push(note(
            "item/completed",
            json!({"turnId": "u1", "item": {"type": "plan", "id": "p1", "text": "1. a\n2. b"}}),
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !rig.host.log().iter().any(|l| l == "plan 1. a\n2. b") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the plan reaches the host");
    }

    /// Nothing here prompts the thread, and the TUI answers its own
    /// approvals: a request reaches no card and gets no answer from here.
    #[tokio::test]
    async fn it_neither_prompts_nor_answers_requests() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        let err = rig
            .mgr
            .submit_human_prompt(&session(), "hi".into())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Codex's own TUI"), "{err}");
        fake.push(json!({"id": 41, "method": "item/commandExecution/requestApproval", "params": {
            "threadId": THREAD, "turnId": "u1", "itemId": "e9", "command": "rm x", "startedAtMs": 1}}));
        // A notification behind it: once it has arrived, so had the request.
        fake.push(note("thread/name/updated", json!({"threadName": "after"})));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !rig.host.log().contains(&"named after".to_string()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!fake.received().iter().any(|v| v["id"] == 41));
        assert!(!rig
            .items()
            .iter()
            .any(|b| matches!(b, ItemBody::Permission { .. })));
        assert!(!methods(&fake).iter().any(|m| m == "turn/start"));
    }

    /// The thread opens sandboxed to its worktree — Codex may change it,
    /// nothing outside it — and nothing here changes the sandbox after.
    #[tokio::test]
    async fn the_thread_opens_sandboxed_to_its_worktree() {
        let (_rig, fake) = open_codex(Host::default(), "thread/start").await;
        let start = fake.wait_for(|v| v["method"] == "thread/start").await;
        assert_eq!(start["params"]["sandbox"], "workspace-write");
        assert!(!methods(&fake).iter().any(|m| m == "thread/settings/update"));
    }
}
