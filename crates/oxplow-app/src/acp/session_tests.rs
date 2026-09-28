//! The session against the scripted fake agent (`oxplow-acp-fake`) over
//! an in-memory pipe, with a recording host in place of `Services`.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxplow_acp_fake::{FakeOptions, FakeState, Shared};
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{DenyLayer, IntentKind, PolicyDecision};
use parking_lot::Mutex;
use tokio::sync::broadcast;

use super::host::AcpHost;
use super::manager::AcpManager;
use super::mapping::AcpIntent;
use super::model::PermissionAnswer;
use super::session::{AcpError, AcpEvent, AcpEventBody, AcpStatus, SessionSpec};
use super::transcript::ItemBody;
use super::wire::{McpHttp, TurnTokens};
use crate::agent_activity::CanonicalToolEvent;

#[derive(Default)]
struct Host {
    /// Deny every worktree write with this reason.
    deny_writes: Option<String>,
    context: Option<String>,
    nudge: Option<String>,
    directive: Option<String>,
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
impl AcpHost for Host {
    async fn check_tool(
        &self,
        _t: &ThreadId,
        _s: &str,
        intent: &AcpIntent,
        _p: &serde_json::Value,
    ) -> PolicyDecision {
        self.log
            .lock()
            .push(format!("check {} {}", intent.label, intent.paths.join(",")));
        match (&self.deny_writes, intent.kind) {
            (Some(r), IntentKind::WorktreeWrite) => PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason: r.clone(),
            },
            _ => PolicyDecision::Allow,
        }
    }
    async fn session_started(&self, _t: &ThreadId, s: &str) {
        self.log.lock().push(format!("started {s}"));
    }
    async fn prompt_context(&self, _t: &ThreadId, _s: &str) -> Option<String> {
        self.context.clone()
    }
    async fn turn_started(&self, _t: &ThreadId, _s: &str, prompt: &str) {
        self.log.lock().push(format!("turn_started {prompt}"));
    }
    async fn tool_finished(
        &self,
        _t: &ThreadId,
        _s: &str,
        e: &CanonicalToolEvent,
    ) -> Option<String> {
        self.log.lock().push(format!("tool {}", e.tool_name));
        self.nudge.clone()
    }
    async fn turn_ended(
        &self,
        _t: &ThreadId,
        _s: &str,
        _prompt: &str,
        tokens: Option<&TurnTokens>,
    ) -> Option<String> {
        self.log.lock().push(format!(
            "turn_ended {}",
            tokens
                .map(|t| format!("{}/{}", t.input, t.output))
                .unwrap_or_default()
        ));
        self.directive.clone()
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

fn spec(cwd: &std::path::Path) -> SessionSpec {
    SessionSpec {
        thread_id: thread(),
        agent: "fake".into(),
        cwd: cwd.to_path_buf(),
        mcp: vec![McpHttp {
            name: "oxplow".into(),
            url: "http://127.0.0.1:1/mcp".into(),
            headers: vec![("Authorization".into(), "Bearer t".into())],
        }],
        resume_session_id: None,
        system_prompt: None,
        system_prompt_via_meta: false,
    }
}

struct Rig {
    mgr: AcpManager,
    fake: Shared,
    events: broadcast::Receiver<AcpEvent>,
    host: Arc<Host>,
    dir: tempfile::TempDir,
}

async fn open_with(
    host: Host,
    fake: Shared,
    opts: FakeOptions,
    spec_fn: impl FnOnce(&mut SessionSpec),
    dir: tempfile::TempDir,
) -> (Rig, Result<(), AcpError>) {
    let mgr = AcpManager::new();
    let host = Arc::new(host);
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    let f = fake.clone();
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, f, opts).await;
    });
    let (cr, cw) = tokio::io::split(client);
    let mut s = spec(dir.path());
    spec_fn(&mut s);
    let r = mgr.open_with_io(host.clone(), s, cw, cr).await;
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
    async fn wait(&mut self, what: &str, pred: impl Fn(&AcpEventBody) -> bool) -> AcpEventBody {
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

    async fn wait_status(&mut self, status: AcpStatus) {
        self.wait(
            &format!("{status:?}"),
            |b| matches!(b, AcpEventBody::Status { status: s } if *s == status),
        )
        .await;
    }

    async fn prompt(&self, text: &str) {
        self.mgr
            .submit_human_prompt(&thread(), text.to_string())
            .await
            .unwrap();
    }

    /// Prompt and wait for the turn to end.
    async fn turn(&mut self, text: &str) {
        self.prompt(text).await;
        self.wait_status(AcpStatus::Idle).await;
    }

    fn items(&self) -> Vec<ItemBody> {
        self.mgr
            .transcript(&thread(), 0)
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
    assert!(rig.host.log().contains(&"turn_ended 10/5".to_string()));
    assert!(rig.host.log().contains(&"tool Bash".to_string()));
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
        .wait("card", |b| matches!(b, AcpEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await
    {
        AcpEventBody::Item { item } => item,
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
        rig.mgr.transcript(&thread(), 0).unwrap().status,
        AcpStatus::AwaitingPermission
    );
    assert_eq!(
        rig.mgr
            .respond_permission(&thread(), request_id.clone(), Some("always".into()))
            .await,
        Err(AcpError::UnknownOption("always".into()))
    );
    rig.mgr
        .respond_permission(&thread(), request_id, Some("allow".into()))
        .await
        .unwrap();
    rig.wait_status(AcpStatus::Idle).await;
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
    rig.wait("card", |b| matches!(b, AcpEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await;
    rig.mgr.cancel(&thread()).unwrap();
    rig.wait_status(AcpStatus::Idle).await;
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
    rig.wait_status(AcpStatus::Running).await;
    assert_eq!(
        rig.mgr.submit_human_prompt(&thread(), "again".into()).await,
        Err(AcpError::TurnInFlight)
    );
    rig.mgr.cancel(&thread()).unwrap();
    rig.wait_status(AcpStatus::Idle).await;
    assert_eq!(rig.prompts().len(), 1);
}

#[tokio::test]
async fn the_directive_is_shown_and_never_sent() {
    let mut rig = open(Host {
        directive: Some("Close the task first.".into()),
        ..Default::default()
    })
    .await;
    rig.turn("fake:say done").await;
    let snap = rig.mgr.transcript(&thread(), 0).unwrap();
    assert_eq!(snap.directive.as_deref(), Some("Close the task first."));
    assert!(rig
        .items()
        .iter()
        .any(|b| matches!(b, ItemBody::Directive { .. })));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.prompts().len(), 1, "the directive must not be sent");
    rig.mgr.dismiss_directive(&thread()).unwrap();
    assert_eq!(rig.mgr.transcript(&thread(), 0).unwrap().directive, None);
}

#[tokio::test]
async fn context_and_nudges_ride_the_next_human_prompt() {
    let mut rig = open(Host {
        context: Some("CTX".into()),
        nudge: Some("NUDGE".into()),
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
    assert_eq!(texts(1), vec!["CTX\n\nNUDGE", "next"]);
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
    let Err(AcpError::Agent(msg)) = r else {
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
    let sid = first.mgr.transcript(&thread(), 0).unwrap();
    assert_eq!(sid.items.len(), 3);
    let session_id = first
        .host
        .log()
        .iter()
        .find_map(|l| l.strip_prefix("started ").map(str::to_string))
        .unwrap();
    first.mgr.close(&thread()).unwrap();

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
        .wait("closed", |b| matches!(b, AcpEventBody::Closed { .. }))
        .await;
    assert!(matches!(closed, AcpEventBody::Closed { reason: Some(_) }));
    assert_eq!(
        rig.mgr.transcript(&thread(), 0).unwrap().status,
        AcpStatus::Stopped
    );
    assert!(rig.host.log().contains(&"interrupted".to_string()));
    assert!(!rig.mgr.is_open(&thread()));
    assert_eq!(
        rig.mgr.submit_human_prompt(&thread(), "x".into()).await,
        Err(AcpError::NotOpen)
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
    assert_eq!(rig.host.count("tool Bash"), 2);
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
    let g1 = first.mgr.transcript(&thread(), 0).unwrap().generation;
    first.mgr.close(&thread()).unwrap();
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
        .open_with_io(first.host.clone(), spec(first.dir.path()), cw, cr)
        .await
        .unwrap();
    let g2 = first.mgr.transcript(&thread(), 0).unwrap().generation;
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
    let mgr = AcpManager::new();
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
        mgr.open_with_io(host.clone(), spec(dir.path()), w1, r1),
        mgr.open_with_io(host.clone(), spec(dir.path()), w2, r2),
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
    rig.mgr.close(&thread()).unwrap();
    assert!(!rig.mgr.is_open(&thread()), "closed at once");
    // Reopen right away (a Restart) while the old actor may still be
    // shutting down.
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, fake, FakeOptions::default()).await;
    });
    let (cr, cw) = tokio::io::split(client);
    rig.mgr
        .open_with_io(rig.host.clone(), spec(rig.dir.path()), cw, cr)
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
    rig.wait("card", |b| matches!(b, AcpEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. })))
        .await;
    rig.wait_status(AcpStatus::AwaitingPermission).await;
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
        .respond_permission(&thread(), request_id, Some("reject".into()))
        .await
        .unwrap();
    let next = rig
        .wait("status", |b| matches!(b, AcpEventBody::Status { .. }))
        .await;
    assert_eq!(
        next,
        AcpEventBody::Status {
            status: AcpStatus::Running
        }
    );
    rig.mgr.cancel(&thread()).unwrap();
    rig.wait_status(AcpStatus::Idle).await;
}
