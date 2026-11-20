//! The session actor against a scripted in-process agent (`Script`, a
//! [`Connection`] the test feeds [`Incoming`] messages) and a recording host
//! in place of `Services`; then a Codex thread on a scripted app-server
//! daemon.

#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{DenyLayer, PolicyDecision};
use parking_lot::Mutex;
use tokio::sync::{mpsc, Notify};

use super::conn::{Connection, Incoming, TurnEnd, TurnTokens};
use super::host::ChatHost;
use super::manager::ChatManager;
use super::model::{ChatUpdate, ToolCall, ToolCallPatch, ToolKind, ToolStatus};
use super::session::{ChatError, SessionSpec};
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
            .push(format!("check {}", tool.paths.join(",")));
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
    async fn turn_started(&self, _t: &ThreadId, _s: &str, prompt: &str, _h: Option<&str>) {
        self.log.lock().push(format!("turn_started {prompt}"));
    }
    async fn tool_started(&self, _t: &ThreadId, _s: &str, tool: &ToolUse) {
        self.log
            .lock()
            .push(format!("tool_started {}", tool.kind.as_str()));
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
        run: Option<&oxplow_domain::hook::TurnRun>,
    ) {
        self.log.lock().push(format!(
            "turn_ended {}{}{}",
            tokens
                .map(|t| format!("{}/{}", t.input, t.output))
                .unwrap_or_default(),
            answer.map(|a| format!(" answer={a}")).unwrap_or_default(),
            run.and_then(|r| r.outcome)
                .map(|o| format!(" outcome={}", o.as_str()))
                .unwrap_or_default()
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
        _spawned_in: Option<&str>,
    ) {
        let what = match change {
            crate::chat::conn::SubagentChange::Started => "started",
            crate::chat::conn::SubagentChange::Finished(_) => "finished",
        };
        self.log.lock().push(format!("subagent {what} {id}"));
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
        cwd: cwd.to_path_buf(),
    }
}

// ---------------------------------------------------------------------
// The scripted agent: the test sends what it says.
// ---------------------------------------------------------------------

#[derive(Default)]
struct Script {
    /// What a continued session replays before `open` returns.
    replay: Mutex<Vec<Incoming>>,
    /// `open` fails with this.
    refuse: Option<String>,
    tx: Mutex<Option<mpsc::UnboundedSender<Incoming>>>,
    opens: Mutex<usize>,
    gone: AtomicBool,
    closed: Notify,
}

impl Script {
    fn connect(self: &Arc<Self>) -> (Arc<dyn Connection>, mpsc::UnboundedReceiver<Incoming>) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.tx.lock() = Some(tx);
        (self.clone(), rx)
    }

    fn send(&self, msg: Incoming) {
        let _ = self.tx.lock().as_ref().unwrap().send(msg);
    }

    fn update(&self, u: ChatUpdate) {
        self.send(Incoming::Update(Box::new(u)));
    }

    fn prompt(&self, text: &str) {
        self.update(ChatUpdate::UserChunk(text.into()));
    }

    fn say(&self, text: &str) {
        self.update(ChatUpdate::AgentChunk(text.into()));
    }

    fn tool(&self, id: &str, kind: ToolKind, path: &str) {
        self.update(ChatUpdate::ToolCall(ToolCall {
            id: id.into(),
            title: format!("{} {path}", kind_title(kind)),
            kind,
            status: ToolStatus::InProgress,
            locations: vec![path.into()],
            ..Default::default()
        }));
    }

    fn finish(&self, id: &str) {
        self.update(ChatUpdate::ToolCallUpdate(ToolCallPatch {
            id: id.into(),
            status: Some(ToolStatus::Completed),
            ..Default::default()
        }));
    }

    fn end(&self, stop_reason: &str, usage: Option<(u64, u64)>) {
        self.send(Incoming::TurnEnded(Ok(TurnEnd {
            stop_reason: stop_reason.into(),
            usage: usage.map(|(input, output)| TurnTokens {
                input,
                output,
                ..Default::default()
            }),
            run: None,
        })));
    }

    /// The agent goes away.
    fn crash(&self) {
        self.gone.store(true, Ordering::SeqCst);
        self.closed.notify_waiters();
    }
}

fn kind_title(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Edit => "Edit",
        ToolKind::Execute => "Run",
        _ => "Tool",
    }
}

#[async_trait]
impl Connection for Script {
    async fn open(&self, _cwd: &std::path::Path) -> Result<String, String> {
        *self.opens.lock() += 1;
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        for msg in self.replay.lock().drain(..) {
            self.send(msg);
        }
        Ok("s1".into())
    }
    async fn closed(&self) {
        while !self.gone.load(Ordering::SeqCst) {
            let notified = self.closed.notified();
            if self.gone.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

struct Rig {
    mgr: ChatManager,
    agent: Arc<Script>,
    host: Arc<Host>,
    dir: tempfile::TempDir,
}

async fn open_with(host: Host, agent: Script) -> (Rig, Result<(), ChatError>) {
    let mgr = ChatManager::new();
    let host = Arc::new(host);
    let agent = Arc::new(agent);
    let dir = tempfile::tempdir().unwrap();
    let (conn, incoming) = agent.connect();
    let r = mgr
        .open_connection(host.clone(), spec(dir.path()), conn, incoming)
        .await;
    (
        Rig {
            mgr,
            agent,
            host,
            dir,
        },
        r,
    )
}

async fn open(host: Host) -> Rig {
    let (rig, r) = open_with(host, Script::default()).await;
    r.unwrap();
    rig
}

impl Rig {
    /// Wait until the host's log holds `n` lines starting with `prefix`.
    async fn until(&self, prefix: &str, n: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.host.count(prefix) < n {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {n} `{prefix}`: {:?}",
                self.host.log()
            )
        });
    }

    /// The log after the session started.
    fn recorded(&self) -> Vec<String> {
        self.host.log()[1..].to_vec()
    }
}

/// A prompt the person typed in the agent's UI opens a turn; a tool call is
/// recorded at its start and its finish (a write checked against the
/// policy as it lands); the turn ends with its tokens and its answer.
#[tokio::test]
async fn a_prompt_in_the_agents_ui_is_a_recorded_turn() {
    let rig = open(Host::default()).await;
    rig.agent.prompt("fix it");
    rig.agent.tool("t1", ToolKind::Edit, "src/a.rs");
    rig.agent.finish("t1");
    rig.agent.say("hel");
    rig.agent.say("lo");
    rig.agent.end("end_turn", Some((3, 4)));
    rig.until("turn_ended", 1).await;
    assert_eq!(
        rig.recorded(),
        [
            "turn_started fix it",
            "tool_started edit",
            "check src/a.rs",
            "tool edit",
            "turn_ended 3/4 answer=hello"
        ]
    );
}

/// The answer is the agent's last run of text: a tool call or a thought
/// ends the text before it.
#[tokio::test]
async fn the_answer_is_the_agents_last_run_of_text() {
    let rig = open(Host::default()).await;
    rig.agent.prompt("go");
    rig.agent.say("looking");
    rig.agent.tool("t1", ToolKind::Execute, "ls");
    rig.agent.finish("t1");
    rig.agent.say("it is ");
    rig.agent.say("there");
    rig.agent.end("end_turn", None);
    rig.agent.prompt("again");
    rig.agent.say("first");
    rig.agent.update(ChatUpdate::ThoughtChunk);
    rig.agent.say("second");
    rig.agent.end("end_turn", None);
    rig.until("turn_ended", 2).await;
    let ends: Vec<String> = rig
        .host
        .log()
        .into_iter()
        .filter(|l| l.starts_with("turn_ended"))
        .collect();
    assert_eq!(
        ends,
        [
            "turn_ended  answer=it is there",
            "turn_ended  answer=second"
        ]
    );
}

/// A turn the agent cancelled is interrupted; one that failed is errored.
#[tokio::test]
async fn a_cancelled_turn_is_interrupted_and_a_failed_one_errored() {
    let rig = open(Host::default()).await;
    rig.agent.prompt("one");
    rig.agent.end("interrupted", None);
    rig.agent.prompt("two");
    rig.agent.send(Incoming::TurnEnded(Err("boom".into())));
    rig.until("turn_ended", 2).await;
    let ends: Vec<String> = rig
        .host
        .log()
        .into_iter()
        .filter(|l| l.starts_with("turn_ended"))
        .collect();
    assert_eq!(
        ends,
        [
            "turn_ended  outcome=interrupted",
            "turn_ended  outcome=errored"
        ]
    );
}

/// What a continued session replays was recorded when it happened: only
/// the name it carries is taken, once the session is up.
#[tokio::test]
async fn a_replay_records_nothing_but_its_name() {
    let agent = Script::default();
    {
        let mut replay = agent.replay.lock();
        replay.push(Incoming::Named("Earlier".into()));
        replay.push(Incoming::Update(Box::new(ChatUpdate::UserChunk(
            "old".into(),
        ))));
        replay.push(Incoming::Update(Box::new(ChatUpdate::ToolCall(ToolCall {
            id: "t0".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            locations: vec!["a.rs".into()],
            ..Default::default()
        }))));
        replay.push(Incoming::TurnEnded(Ok(TurnEnd {
            stop_reason: "end_turn".into(),
            usage: None,
            run: None,
        })));
    }
    let (rig, r) = open_with(Host::default(), agent).await;
    r.unwrap();
    assert_eq!(rig.host.log(), ["started s1", "named Earlier"]);
}

/// A write the agent made that the policy denies is on the record as
/// checked, and still recorded as made.
#[tokio::test]
async fn a_denied_write_is_checked_and_still_recorded() {
    let rig = open(Host {
        deny_writes: Some("another stream's worktree".into()),
        ..Host::default()
    })
    .await;
    rig.agent.prompt("go");
    rig.agent.tool("t1", ToolKind::Edit, "../other/a.rs");
    rig.agent.finish("t1");
    rig.until("tool edit", 1).await;
    assert!(rig.host.log().contains(&"check ../other/a.rs".to_string()));
}

/// Tool calls are recorded by their id within one run: a finish is
/// recorded once, and the same id in a later turn is the same call.
#[tokio::test]
async fn a_finish_is_recorded_once() {
    let rig = open(Host::default()).await;
    rig.agent.prompt("go");
    rig.agent.tool("t1", ToolKind::Execute, "ls");
    rig.agent.finish("t1");
    rig.agent.finish("t1");
    rig.agent.end("end_turn", None);
    rig.until("turn_ended", 1).await;
    assert_eq!(rig.host.count("tool shell"), 1, "{:?}", rig.host.log());
    assert_eq!(rig.host.count("tool_started"), 1);
}

/// An agent that goes away interrupts the session, which is no longer
/// open.
#[tokio::test]
async fn an_agent_that_goes_away_interrupts_the_session() {
    let rig = open(Host::default()).await;
    rig.agent.crash();
    rig.until("interrupted", 1).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while rig.mgr.is_open(&session()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_agent_that_refuses_to_open_is_reported() {
    let (rig, r) = open_with(
        Host::default(),
        Script {
            refuse: Some("no such thread".into()),
            ..Script::default()
        },
    )
    .await;
    let Err(ChatError::Agent(msg)) = r else {
        panic!("expected an agent error, got {r:?}");
    };
    assert!(msg.contains("no such thread"), "{msg}");
    assert!(!rig.mgr.is_open(&session()));
    assert_eq!(rig.host.count("interrupted"), 0);
}

#[tokio::test]
async fn concurrent_opens_start_one_agent() {
    let mgr = ChatManager::new();
    let host = Arc::new(Host::default());
    let dir = tempfile::tempdir().unwrap();
    let agent = Arc::new(Script::default());
    let (c1, r1) = agent.connect();
    let (c2, r2) = agent.connect();
    let (a, b) = tokio::join!(
        mgr.open_connection(host.clone(), spec(dir.path()), c1, r1),
        mgr.open_connection(host.clone(), spec(dir.path()), c2, r2),
    );
    a.unwrap();
    b.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(*agent.opens.lock(), 1, "one agent session, not two");
    assert_eq!(host.count("started"), 1);
}

#[tokio::test]
async fn close_is_immediate_and_a_replaced_sessions_teardown_records_nothing() {
    let rig = open(Host::default()).await;
    rig.mgr.close(&session()).unwrap();
    assert!(!rig.mgr.is_open(&session()), "closed at once");
    // Reopen right away (a Restart) while the old actor may still be
    // shutting down.
    let again = Arc::new(Script::default());
    let (conn, incoming) = again.connect();
    rig.mgr
        .open_connection(rig.host.clone(), spec(rig.dir.path()), conn, incoming)
        .await
        .unwrap();
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

/// The agent waiting on the person in its own UI is said, and cleared.
#[tokio::test]
async fn a_wait_in_the_agents_ui_is_said_and_cleared() {
    let rig = open(Host::default()).await;
    rig.agent
        .send(Incoming::Waiting(Some("Approve rm?".into())));
    rig.agent.send(Incoming::Waiting(None));
    rig.until("awaiting", 2).await;
    assert_eq!(rig.recorded(), ["awaiting true", "awaiting false"]);
}

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
        (
            Rig {
                mgr,
                agent: Arc::new(Script::default()),
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
        // The goal is set paused: an active goal makes Codex run a turn on
        // its own with no prompt (recorded on 0.167.1 and checked live: an
        // active goal starts one, a paused goal still writes the rollout and
        // starts none), which costs quota and is nobody's turn.
        let goal = fake
            .received()
            .into_iter()
            .find(|v| v["method"] == "thread/goal/set")
            .unwrap();
        assert_eq!(goal["params"]["status"], "paused");
        let (rig, fake) = open_codex(Host::default(), "thread/resume").await;
        assert!(!methods(&fake).iter().any(|m| m.starts_with("thread/goal")));
        // Named once the session it names is recorded.
        assert_eq!(rig.host.log(), ["started thr-x", "named Earlier work"]);
    }

    /// A turn Codex starts by itself (no `userMessage`) is nobody's prompt:
    /// its calls and end are recorded, but no person's turn opens for it.
    #[tokio::test]
    async fn a_turn_with_no_prompt_is_not_a_persons_turn() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        fake.push(note("turn/started", json!({"turn": {"id": "u0"}})));
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u0", "status": "completed"}}),
        ));
        for _ in 0..50 {
            if rig.host.count("turn_ended") > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(rig.host.count("turn_ended"), 1, "{:?}", rig.host.log());
        assert_eq!(rig.host.count("turn_started"), 0, "{:?}", rig.host.log());
    }

    /// A recorded Codex session (a `tests/fixtures/codex/*.jsonl` file) pushed
    /// through a real session on the scripted daemon, as the observer's
    /// connection got it: every message the daemon sent, its thread renamed
    /// to this session's. Returns the host's log once it stops growing.
    async fn replay_fixture(text: &str) -> Vec<String> {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        let lines: Vec<serde_json::Value> = text
            .lines()
            .skip(1)
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let ours = lines
            .iter()
            .find_map(|v| v.pointer("/response/thread/id")?.as_str())
            .unwrap()
            .to_string();
        for v in &lines {
            if v.get("method").is_some() {
                let said = v.to_string().replace(&ours, THREAD);
                let mut m: serde_json::Value = serde_json::from_str(&said).unwrap();
                for k in ["conn", "purpose", "emittedAtMs"] {
                    m.as_object_mut().unwrap().remove(k);
                }
                fake.push(m);
            }
        }
        let mut seen = 0;
        let mut steady = 0;
        while steady < 8 {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let now = rig.host.log().len();
            steady = if now == seen { steady + 1 } else { 0 };
            seen = now;
        }
        rig.host.log()
    }

    /// Codex starts an ephemeral naming thread after every user message (it
    /// arrives as `thread/started`, goes idle or `systemError`, and closes a
    /// minute later), and a thread of an earlier session on the same daemon
    /// arrives as `notLoaded` and `thread/closed`. Neither is this session's:
    /// no session, turn or tool call of the host's comes of them. Replays each
    /// recording through a session: one session, and as many turns as the
    /// person's prompts.
    #[tokio::test]
    async fn another_thread_on_the_daemon_is_not_this_sessions() {
        for (name, text, turns) in [
            (
                "approval",
                include_str!("../../tests/fixtures/codex/approval.jsonl"),
                1,
            ),
            (
                "interrupt",
                include_str!("../../tests/fixtures/codex/interrupt.jsonl"),
                1,
            ),
            (
                "background_terminal",
                include_str!("../../tests/fixtures/codex/background_terminal.jsonl"),
                1,
            ),
            (
                "subagent",
                include_str!("../../tests/fixtures/codex/subagent.jsonl"),
                1,
            ),
            (
                "failed_turn",
                include_str!("../../tests/fixtures/codex/failed_turn.jsonl"),
                1,
            ),
        ] {
            assert!(
                text.contains("\"ephemeral\": true"),
                "{name} shows the naming thread"
            );
            let log = replay_fixture(text).await;
            assert_eq!(
                log.iter().filter(|l| l.starts_with("started")).count(),
                1,
                "{name}: {log:?}"
            );
            assert_eq!(
                log.iter().filter(|l| l.starts_with("turn_started")).count(),
                turns,
                "{name}: {log:?}"
            );
            assert!(
                log.iter().filter(|l| l.starts_with("turn_ended")).count() <= 1,
                "{name}: {log:?}"
            );
            assert_eq!(
                log.iter().filter(|l| l.starts_with("named")).count(),
                usize::from(name != "failed_turn"),
                "only this thread's name counts: {name}: {log:?}"
            );
        }
    }

    /// The recorded failed turn through a session: one turn, ended once,
    /// `errored` with the final error; the retries in between end nothing.
    #[tokio::test]
    async fn a_recorded_failed_turn_is_one_errored_turn() {
        let log =
            replay_fixture(include_str!("../../tests/fixtures/codex/failed_turn.jsonl")).await;
        let ended: Vec<&String> = log.iter().filter(|l| l.starts_with("turn_ended")).collect();
        assert_eq!(ended, ["turn_ended  outcome=errored"], "{log:?}");
        assert!(!log.iter().any(|l| l.starts_with("awaiting")), "{log:?}");
    }

    /// Recorded on 0.167.1: Esc ends the turn `interrupted`, the command
    /// goes on as a background terminal, and `/stop` fails its call later. The
    /// late failure is recorded on its call; no turn starts or ends for it.
    #[tokio::test]
    async fn a_late_failed_call_after_an_interrupted_turn_opens_no_turn() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        let cmd = |status: &str| {
            note(
                if status == "inProgress" {
                    "item/started"
                } else {
                    "item/completed"
                },
                json!({"item": {"type": "commandExecution", "id": "c1", "command": "sleep 120",
                    "status": status, "commandActions": [{"type": "unknown", "command": "sleep 120"}],
                    "exitCode": if status == "failed" { json!(-1) } else { json!(null) }}}),
            )
        };
        fake.push(note("turn/started", json!({"turn": {"id": "u1"}})));
        fake.push(note("item/completed", json!({"item": {"type": "userMessage", "id": "m1", "content": [{"type": "text", "text": "sleep"}]}})));
        fake.push(cmd("inProgress"));
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u1", "status": "interrupted", "error": null}}),
        ));
        fake.push(cmd("failed"));
        for _ in 0..100 {
            if rig.host.count("tool shell") > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let log = rig.host.log();
        let tail: Vec<&String> = log.iter().filter(|l| !l.starts_with("started")).collect();
        assert_eq!(
            tail,
            [
                "turn_started sleep",
                "tool_started shell",
                "turn_ended  outcome=interrupted",
                "tool shell"
            ],
            "{log:?}"
        );
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
        let (rig, fake) = open_codex(
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
        rig.until("compacted", 1).await;
        injected_is(&fake, 1).await;
        fake.push(note(
            "turn/completed",
            json!({"turn": {"id": "u1", "status": "completed"}}),
        ));
        rig.until("turn_ended", 1).await;
        injected_is(&fake, 2).await;
        assert!(rig.host.log().contains(&"compacted".to_string()));
    }

    /// A context the daemon refused is not delivered: nothing is recorded
    /// as sent, so the next chance (the end of a turn) carries it whole.
    #[tokio::test]
    async fn a_refused_inject_loses_nothing() {
        let (rig, fake) = open_codex_failing(
            Host {
                context: Some("<context-basket>x</context-basket>".into()),
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
        rig.until("turn_ended", 1).await;
        injected_is(&fake, 2).await;
        assert_eq!(
            rig.host
                .log()
                .iter()
                .filter(|l| *l == "delivered <context-basket>x</context-basket>")
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
                context: Some("<context-basket>y</context-basket>".into()),
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
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
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
        rig.until("turn_ended", 1).await;
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

    /// Codex's daemon updates itself in the middle of a session (0.166.0 to
    /// 0.167.1, recorded: the restart killed a background process and its
    /// `item/completed` never came). The observer ends with the connection;
    /// the host is told, so the open turn ends interrupted (and with it
    /// the call it left running, which is abandoned, never running for ever),
    /// and the next open resumes the thread on the daemon that came back.
    #[tokio::test]
    async fn a_daemon_restart_mid_turn_interrupts_the_turn_and_the_call_in_flight() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
        fake.push(note("turn/started", json!({"turn": {"id": "u1"}})));
        fake.push(note(
            "item/completed",
            json!({"item": {"type": "userMessage", "id": "m1", "content": [{"type": "text", "text": "wait"}]}}),
        ));
        fake.push(note(
            "item/started",
            json!({"item": {"type": "commandExecution", "id": "c1", "command": "sleep 40",
                "status": "inProgress", "commandActions": [{"type": "unknown", "command": "sleep 40"}]}}),
        ));
        for _ in 0..100 {
            if rig.host.count("turn_started") > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(rig.host.count("turn_started"), 1, "{:?}", rig.host.log());
        fake.stop();
        drop(fake);
        tokio::time::timeout(Duration::from_secs(5), async {
            while rig.mgr.is_open(&session()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the observer ends with its daemon");
        let log = rig.host.log();
        assert_eq!(
            log.iter().filter(|l| *l == "interrupted").count(),
            1,
            "the host ends the open turn: {log:?}"
        );
        assert_eq!(
            rig.host.count("tool "),
            0,
            "the call never finished: {log:?}"
        );
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

    /// Nothing here prompts the thread, and the TUI answers its own
    /// approvals: a request reaches no card and gets no answer from here.
    #[tokio::test]
    async fn it_neither_prompts_nor_answers_requests() {
        let (rig, fake) = open_codex(Host::default(), "thread/start").await;
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
