//! The session against the scripted fake agent (`oxplow-acp-fake`) over
//! an in-memory pipe, with a recording host in place of `Services`.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxplow_acp_fake::{FakeOptions, FakeState, Shared};
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{DenyLayer, PolicyDecision};
use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::acp::{AcpOpen, McpHttp};
use super::conn::TurnTokens;
use super::host::ChatHost;
use super::manager::ChatManager;
use super::model::PermissionAnswer;
use super::session::{ChatError, ChatEvent, ChatEventBody, ChatStatus, SessionSpec};
use super::transcript::ItemBody;
use oxplow_domain::agent::tool::{ToolKind as Kind, ToolUse};

#[derive(Default)]
struct Host {
    /// Deny every worktree write with this reason.
    deny_writes: Option<String>,
    /// The thread isn't its stream's writer (flipped to demote it).
    read_only: std::sync::atomic::AtomicBool,
    context: Option<String>,
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
    async fn writable(&self, _t: &ThreadId) -> bool {
        !self.read_only.load(std::sync::atomic::Ordering::SeqCst)
    }
    async fn prompt_context(&self, _t: &ThreadId, _s: &str) -> Option<String> {
        self.context.clone()
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

fn acp_open() -> AcpOpen {
    AcpOpen {
        agent: "fake".into(),
        mcp: vec![McpHttp {
            name: "oxplow".into(),
            url: "http://127.0.0.1:1/mcp".into(),
            headers: vec![("Authorization".into(), "Bearer t".into())],
        }],
        ..AcpOpen::default()
    }
}

struct Rig {
    mgr: ChatManager,
    fake: Shared,
    events: broadcast::Receiver<ChatEvent>,
    host: Arc<Host>,
    dir: tempfile::TempDir,
}

async fn open_with(
    host: Host,
    fake: Shared,
    opts: FakeOptions,
    open_fn: impl FnOnce(&mut AcpOpen),
    dir: tempfile::TempDir,
) -> (Rig, Result<(), ChatError>) {
    let mgr = ChatManager::new();
    let host = Arc::new(host);
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    let f = fake.clone();
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, f, opts).await;
    });
    let (cr, cw) = tokio::io::split(client);
    let mut open = acp_open();
    open_fn(&mut open);
    let r = mgr
        .open_with_io(host.clone(), spec(dir.path()), open, cw, cr)
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
        |_| {},
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    rig
}

impl Rig {
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

    fn prompts(&self) -> Vec<serde_json::Value> {
        self.fake.lock().unwrap().prompts.clone()
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
        deny_writes: Some("read-only thread".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:edit /w/a.rs").await;
    let items = rig.items();
    assert!(!items
        .iter()
        .any(|b| matches!(b, ItemBody::Permission { .. })));
    assert!(items.iter().any(
        |b| matches!(b, ItemBody::PolicyDenied { reason, .. } if reason == "read-only thread")
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
    let texts = |i: usize| -> Vec<String> {
        prompts[i]["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["text"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(texts(0), vec!["CTX", "fake:bash cargo test"]);
    assert_eq!(texts(1), vec!["CTX", "next"]);
}

#[tokio::test]
async fn the_system_prompt_goes_in_meta_or_ahead_of_the_first_prompt() {
    let (mut meta, r) = open_with(
        Host::default(),
        Shared::default(),
        FakeOptions::default(),
        |s| {
            s.system_prompt = Some("SYS".into());
            s.system_prompt_via_meta = true;
        },
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    meta.turn("a").await;
    let new = meta.fake.lock().unwrap().new_sessions[0].clone();
    assert_eq!(new["_meta"]["systemPrompt"]["append"], "SYS");
    assert_eq!(new["mcpServers"][0]["type"], "http");
    assert_eq!(new["mcpServers"][0]["headers"][0]["name"], "Authorization");
    assert_eq!(meta.prompts()[0]["prompt"].as_array().unwrap().len(), 1);

    let (mut inline, r) = open_with(
        Host::default(),
        Shared::default(),
        FakeOptions::default(),
        |s| s.system_prompt = Some("SYS".into()),
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    inline.turn("a").await;
    inline.turn("b").await;
    let p = inline.prompts();
    assert_eq!(p[0]["prompt"][0]["text"], "SYS");
    assert_eq!(p[1]["prompt"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn an_agent_without_http_mcp_is_refused_clearly() {
    let (_rig, r) = open_with(
        Host::default(),
        Shared::default(),
        FakeOptions {
            mcp_http: false,
            ..Default::default()
        },
        |_| {},
        tempfile::tempdir().unwrap(),
    )
    .await;
    let Err(ChatError::Agent(msg)) = r else {
        panic!("expected a refusal")
    };
    assert!(msg.contains("HTTP MCP"), "{msg}");
}

#[tokio::test]
async fn a_load_replay_rebuilds_the_transcript_and_records_nothing() {
    let fake: Shared = Arc::new(std::sync::Mutex::new(FakeState::default()));
    let mut first = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        |_| {},
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
        FakeOptions::default(),
        |s| s.resume_session_id = Some(session_id.clone()),
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    assert_eq!(fake.lock().unwrap().loads.len(), 1);
    assert_eq!(
        fake.lock().unwrap().new_sessions.len(),
        1,
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
    let fake: Shared = Arc::new(std::sync::Mutex::new(FakeState::default()));
    let (first, r) = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        |_| {},
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    let g1 = first.mgr.transcript(&session(), 0).unwrap().generation;
    first.mgr.close(&session()).unwrap();
    // Reopening on the same manager (a Restart) starts a new generation.
    let mut events = first.mgr.subscribe();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, fake, FakeOptions::default()).await;
    });
    let (cr, cw) = tokio::io::split(client);
    first
        .mgr
        .open_with_io(
            first.host.clone(),
            spec(first.dir.path()),
            acp_open(),
            cw,
            cr,
        )
        .await
        .unwrap();
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
    let fake: Shared = Arc::new(std::sync::Mutex::new(FakeState::default()));
    let mgr = ChatManager::new();
    let host = Arc::new(Host::default());
    let dir = tempfile::tempdir().unwrap();
    let transport = || {
        let (client, agent) = tokio::io::duplex(1 << 16);
        let (ar, aw) = tokio::io::split(agent);
        let f = fake.clone();
        tokio::spawn(async move {
            let _ = oxplow_acp_fake::serve(ar, aw, f, FakeOptions::default()).await;
        });
        tokio::io::split(client)
    };
    let (r1, w1) = transport();
    let (r2, w2) = transport();
    let (a, b) = tokio::join!(
        mgr.open_with_io(host.clone(), spec(dir.path()), acp_open(), w1, r1),
        mgr.open_with_io(host.clone(), spec(dir.path()), acp_open(), w2, r2),
    );
    a.unwrap();
    b.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        fake.lock().unwrap().new_sessions.len(),
        1,
        "one agent session, not two"
    );
    assert_eq!(host.count("started"), 1);
}

#[tokio::test]
async fn close_is_immediate_and_a_replaced_sessions_teardown_records_nothing() {
    let fake: Shared = Arc::new(std::sync::Mutex::new(FakeState::default()));
    let (rig, r) = open_with(
        Host::default(),
        fake.clone(),
        FakeOptions::default(),
        |_| {},
        tempfile::tempdir().unwrap(),
    )
    .await;
    r.unwrap();
    rig.mgr.close(&session()).unwrap();
    assert!(!rig.mgr.is_open(&session()), "closed at once");
    // Reopen right away (a Restart) while the old actor may still be
    // shutting down.
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, fake, FakeOptions::default()).await;
    });
    let (cr, cw) = tokio::io::split(client);
    rig.mgr
        .open_with_io(rig.host.clone(), spec(rig.dir.path()), acp_open(), cw, cr)
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
    use std::sync::atomic::Ordering;

    const THREAD: &str = "thr-x";

    /// A daemon whose `thread/start` opens `thr-x` and whose
    /// `thread/resume` opens it named.
    fn daemon(home: &std::path::Path) -> FakeAppServer {
        FakeAppServer::start(home, |method, _| match method {
            "thread/start" => Ok(json!({"thread": {"id": THREAD, "name": null}})),
            "thread/resume" => Ok(json!({"thread": {"id": THREAD, "name": "Earlier work"}})),
            "thread/goal/set" | "thread/goal/clear" | "thread/settings/update" => Ok(json!({})),
            other => Err(format!("no {other}")),
        })
    }

    async fn open_codex(host: Host, method: &str) -> (Rig, FakeAppServer) {
        // Short: a unix socket's path has a length limit.
        let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
        let fake = daemon(dir.path());
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

    /// The thread opens sandboxed to what it may write, and follows a
    /// promotion or demotion — only a change, since a sandbox change
    /// restarts the thread's MCP servers.
    #[tokio::test]
    async fn the_sandbox_follows_the_threads_write_access() {
        let host = Host::default();
        host.read_only.store(true, Ordering::SeqCst);
        let (rig, fake) = open_codex(host, "thread/start").await;
        let start = fake.wait_for(|v| v["method"] == "thread/start").await;
        assert_eq!(start["params"]["sandbox"], "read-only");
        rig.mgr.sync_access();
        rig.host.read_only.store(false, Ordering::SeqCst);
        rig.mgr.sync_access();
        let update = fake
            .wait_for(|v| v["method"] == "thread/settings/update")
            .await;
        assert_eq!(update["params"]["threadId"], THREAD);
        assert_eq!(
            update["params"]["sandboxPolicy"],
            json!({"type": "workspaceWrite"})
        );
        rig.mgr.sync_access();
        rig.host.read_only.store(true, Ordering::SeqCst);
        rig.mgr.sync_access();
        let updates = || {
            fake.received()
                .into_iter()
                .filter(|v| v["method"] == "thread/settings/update")
                .collect::<Vec<_>>()
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while updates().len() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let updates = updates();
        assert_eq!(updates.len(), 2, "one per change");
        assert_eq!(
            updates[1]["params"]["sandboxPolicy"],
            json!({"type": "readOnly"})
        );
    }
}
