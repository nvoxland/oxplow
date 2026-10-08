//! ACP sessions against real `Services` (tsk337): an ACP turn must land in
//! the same tables a hooked terminal turn does. The fake agent runs
//! in-process over a pipe, and once as its real binary.

#![allow(clippy::unwrap_used)]

use oxplow_tasks::work_item_ref;
use std::process::Command as Proc;
use std::sync::Arc;
use std::time::Duration;

use oxplow_acp_fake::{FakeOptions, Shared};
use oxplow_app::acp::host::ServicesAcpHost;
use oxplow_app::acp::manager::Launch;
use oxplow_app::acp::session::{AcpEventBody, AcpStatus, SessionSpec};
use oxplow_app::acp::transcript::ItemBody;
use oxplow_app::Services;
use oxplow_db::semantic_layer::SqlCell;
use oxplow_domain::stores::{AgentSessionStore, AgentTurnStore, StreamStore, ThreadStore};
use oxplow_domain::{
    AgentKind, Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp,
};
use oxplow_tasks::TaskId;
use oxplow_tasks::TaskStore;
use oxplow_tasks::{Task, TaskActorKind, TaskPriority, TaskStatus};

async fn boot() -> (Arc<Services>, std::path::PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(Proc::new("git")
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "test"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    let root = dir.path().canonicalize().unwrap();
    let svc = Arc::new(Services::in_memory(&root).unwrap());
    (svc, root, dir)
}

async fn seed(svc: &Services, root: &std::path::Path, status: ThreadStatus) -> ThreadId {
    let now = Timestamp::from_unix_ms(1);
    let stream = Stream {
        id: StreamId::new(1),
        kind: StreamKind::Primary,
        title: "p".into(),
        branch: "main".into(),
        branch_ref: "refs/heads/main".into(),
        branch_source: "main".into(),
        worktree_path: root.to_string_lossy().into(),
        working_pane: String::new(),
        talking_pane: String::new(),
        working_session_id: String::new(),
        talking_session_id: String::new(),
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    svc.stream_store.upsert(&stream).await.unwrap();
    let thread = Thread {
        id: ThreadId::new(1),
        stream_id: stream.id,
        title: "t".into(),
        status,
        sort_index: 0,
        summary: String::new(),
        summary_updated_at: None,
        closed_at: None,
        custom_prompt: None,
        created_at: now,
        updated_at: now,
        archived_at: None,
    };
    svc.thread_store.upsert(&thread).await.unwrap();
    svc.agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::of(
            thread.id,
            AgentKind::Acp,
            Some("fake".into()),
        ))
        .await
        .unwrap();
    thread.id
}

/// The thread's agent session (the one `seed` opened).
async fn session_of(svc: &Services, thread: ThreadId) -> oxplow_domain::AgentSessionId {
    svc.agent_session_store
        .newest_for_thread(thread)
        .await
        .unwrap()
        .unwrap()
        .id
}

async fn claim_task(svc: &Services, thread: ThreadId) {
    let now = Timestamp::from_unix_ms(1);
    let task = svc
        .task_store
        .insert(&Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread),
            parent_id: None,
            title: "ship it".into(),
            description: "d".into(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        })
        .await
        .unwrap();
    use oxplow_app::EffortStore as _;
    svc.effort_store
        .start(&work_item_ref(task), &thread, None)
        .await
        .unwrap();
}

fn spec(
    session: oxplow_domain::AgentSessionId,
    thread: ThreadId,
    root: &std::path::Path,
) -> SessionSpec {
    SessionSpec {
        session_id: session,
        thread_id: thread,
        agent: "fake".into(),
        cwd: root.to_path_buf(),
        mcp: vec![],
        resume_session_id: None,
        system_prompt: None,
        system_prompt_via_meta: false,
    }
}

/// Open the thread's (first) session's agent in process.
async fn open_in_process(
    svc: &Arc<Services>,
    thread: ThreadId,
    root: &std::path::Path,
) -> oxplow_domain::AgentSessionId {
    let session = session_of(svc, thread).await;
    open_session_in_process(svc, session, thread, root).await;
    session
}

/// Open agent session `session`'s fake ACP agent in process.
async fn open_session_in_process(
    svc: &Arc<Services>,
    session: oxplow_domain::AgentSessionId,
    thread: ThreadId,
    root: &std::path::Path,
) {
    let (client, agent) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(agent);
    tokio::spawn(async move {
        let _ = oxplow_acp_fake::serve(ar, aw, Shared::default(), FakeOptions::default()).await;
    });
    let (cr, cw) = tokio::io::split(client);
    let host = Arc::new(ServicesAcpHost::new(svc, Some(StreamId::new(1)), session));
    svc.acp
        .open_with_io(host, spec(session, thread, root), cw, cr)
        .await
        .unwrap();
}

async fn wait_for(
    rx: &mut tokio::sync::broadcast::Receiver<oxplow_app::acp::session::AcpEvent>,
    pred: impl Fn(&AcpEventBody) -> bool,
) -> AcpEventBody {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let e = rx.recv().await.unwrap();
            if pred(&e.body) {
                return e.body;
            }
        }
    })
    .await
    .expect("timed out")
}

fn items(svc: &Services, session: oxplow_domain::AgentSessionId) -> Vec<ItemBody> {
    svc.acp
        .transcript(&session, 0)
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.body)
        .collect()
}

#[tokio::test]
async fn an_acp_edit_is_recorded_like_a_hooked_one() {
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Active).await;
    claim_task(&svc, thread).await;
    let session = open_in_process(&svc, thread, &root).await;
    // After open: the startup `Idle` must not read as a finished turn.
    let mut rx = svc.acp.subscribe();

    let target = root.join("src/x.rs");
    svc.acp
        .submit_human_prompt(&session, format!("fake:edit {}", target.display()))
        .await
        .unwrap();
    let AcpEventBody::Item { item } = wait_for(&mut rx, |b| {
        matches!(b, AcpEventBody::Item { item } if matches!(item.body, ItemBody::Permission { .. }))
    })
    .await
    else {
        unreachable!()
    };
    let ItemBody::Permission { request_id, .. } = item.body else {
        unreachable!()
    };
    svc.acp
        .respond_permission(&session, request_id, Some("allow".into()))
        .await
        .unwrap();
    wait_for(&mut rx, |b| {
        matches!(
            b,
            AcpEventBody::Status {
                status: AcpStatus::Idle
            }
        )
    })
    .await;

    // The tool-call row, as v_tool_call reads it (written by a pump reactor).
    svc.event_pump.run_once().await.unwrap();
    let layer = svc.sql.clone();
    let rows = layer
        .query_sql(
            "SELECT tool, path FROM v_tool_call WHERE thread_id = ?1",
            vec![SqlCell::Int(thread.value())],
            None,
        )
        .await
        .unwrap()
        .rows;
    assert_eq!(
        rows,
        vec![vec![
            SqlCell::Text("Edit".into()),
            SqlCell::Text("src/x.rs".into())
        ]]
    );
    // The effort claimed the file.
    let files = layer
        .query_sql("SELECT path FROM v_effort_file", vec![], None)
        .await
        .unwrap()
        .rows;
    assert!(
        files.contains(&vec![SqlCell::Text("src/x.rs".into())]),
        "{files:?}"
    );
    // The log saw the whole turn, and the turn is closed.
    let kinds: Vec<String> = svc
        .event_log_store
        .recent("agent", Some(thread), None, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.envelope.event_type)
        .collect();
    for k in [
        "agent.session.started",
        "agent.prompt.submitted",
        "agent.tool.requested",
        "agent.tool.finished",
        "agent.turn.ended",
    ] {
        assert!(kinds.iter().any(|t| t == k), "{k} missing from {kinds:?}");
    }
    assert!(svc
        .agent_turn_store
        .list_open(&thread)
        .await
        .unwrap()
        .is_empty());
    // The session is remembered for the next resume.
    let s = svc
        .agent_session_store
        .newest_for_thread(thread)
        .await
        .unwrap()
        .unwrap();
    assert!(s.resume_session_id.starts_with("fake-session-"));
}

#[tokio::test]
async fn a_read_only_thread_is_refused_by_the_write_guard() {
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Queued).await;
    let session = open_in_process(&svc, thread, &root).await;
    // After open: the startup `Idle` must not read as a finished turn.
    let mut rx = svc.acp.subscribe();
    svc.acp
        .submit_human_prompt(
            &session,
            format!("fake:edit {}", root.join("a.rs").display()),
        )
        .await
        .unwrap();
    wait_for(&mut rx, |b| {
        matches!(
            b,
            AcpEventBody::Status {
                status: AcpStatus::Idle
            }
        )
    })
    .await;
    let items = items(&svc, session);
    assert!(!items
        .iter()
        .any(|b| matches!(b, ItemBody::Permission { .. })));
    let reason = items
        .iter()
        .find_map(|b| match b {
            ItemBody::PolicyDenied { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .unwrap();
    assert!(reason.contains("read-only"), "{reason}");
}

/// The fake's binary, built next to this test binary by a workspace
/// build; built on demand otherwise.
fn fake_bin() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap();
    let bin = dir.join("oxplow-acp-fake");
    if !bin.exists() {
        let ok = Proc::new(env!("CARGO"))
            .args([
                "build",
                "-q",
                "-p",
                "oxplow-acp-fake",
                "--bin",
                "oxplow-acp-fake",
            ])
            .env("CARGO_TARGET_DIR", dir.parent().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok, "building oxplow-acp-fake failed");
    }
    bin
}

#[tokio::test]
async fn the_agent_process_runs_and_a_crash_stops_the_thread() {
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Active).await;
    let session = session_of(&svc, thread).await;
    let host = Arc::new(ServicesAcpHost::new(&svc, Some(StreamId::new(1)), session));
    svc.acp
        .open(
            host,
            spec(session, thread, &root),
            Launch {
                program: fake_bin(),
                args: vec![],
                env: vec![],
            },
        )
        .await
        .unwrap();
    let mut rx = svc.acp.subscribe();
    svc.acp
        .submit_human_prompt(&session, "fake:say from a process".into())
        .await
        .unwrap();
    wait_for(&mut rx, |b| {
        matches!(
            b,
            AcpEventBody::Status {
                status: AcpStatus::Idle
            }
        )
    })
    .await;
    assert!(items(&svc, session)
        .iter()
        .any(|b| matches!(b, ItemBody::Agent { text } if text == "from a process")));

    svc.acp
        .submit_human_prompt(&session, "fake:crash".into())
        .await
        .unwrap();
    wait_for(&mut rx, |b| matches!(b, AcpEventBody::Closed { .. })).await;
    assert!(!svc.acp.is_open(&session));
    let status = svc
        .agent_status_store
        .get(&thread, Some(session_of(&svc, thread).await))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.state, oxplow_domain::AgentStatusState::Stopped);
    assert!(svc
        .agent_turn_store
        .list_open(&thread)
        .await
        .unwrap()
        .is_empty());
}

/// Two ACP sessions in one thread are two agents: a prompt in one is
/// only its transcript's, and closing one leaves the other running.
#[tokio::test]
async fn two_acp_sessions_in_a_thread_run_apart() {
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Active).await;
    let a = open_in_process(&svc, thread, &root).await;
    let b = svc
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::of(
            thread,
            AgentKind::Acp,
            Some("fake".into()),
        ))
        .await
        .unwrap()
        .id;
    open_session_in_process(&svc, b, thread, &root).await;
    let mut rx = svc.acp.subscribe();
    svc.acp
        .submit_human_prompt(&a, "hello".into())
        .await
        .unwrap();
    wait_for(&mut rx, |b| {
        matches!(
            b,
            AcpEventBody::Status {
                status: AcpStatus::Idle
            }
        )
    })
    .await;
    let said = |items: Vec<ItemBody>| {
        items
            .into_iter()
            .any(|i| matches!(i, ItemBody::User { .. }))
    };
    assert!(said(items(&svc, a)));
    assert!(!said(items(&svc, b)));
    svc.acp.close(&a).unwrap();
    assert!(!svc.acp.is_open(&a));
    assert!(svc.acp.is_open(&b));
}

#[tokio::test]
async fn closing_an_acp_thread_stops_its_session() {
    // Every session's process stops with its thread: the ACP agent and a
    // terminal session's PTY.
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Queued).await;
    let session = open_in_process(&svc, thread, &root).await;
    assert!(svc.acp.is_open(&session));
    let terminal = svc
        .agent_session_store
        .open(&oxplow_domain::agent_session::NewAgentSession::of(
            thread,
            AgentKind::Claude,
            None,
        ))
        .await
        .unwrap()
        .id;
    let key = oxplow_app::terminal_sessions::agent_pane_key(terminal);
    svc.terminal_sessions
        .attach_or_create_for_agent(key.clone(), None, 80, 24, |c, r| {
            oxplow_app::terminal_sessions::SpawnRequest {
                command: "sleep".into(),
                args: vec!["30".into()],
                cwd: root.clone(),
                env: vec![],
                env_remove: vec![],
                cols: c,
                rows: r,
            }
        })
        .await
        .unwrap();
    svc.commands
        .run(
            &oxplow_domain::Actor::Human,
            oxplow_app::commands::thread::CLOSE,
            serde_json::json!({ "thread": oxplow_domain::refs::build::thread_ref(thread) }),
            false,
        )
        .await
        .unwrap();
    assert!(
        !svc.acp.is_open(&session),
        "the agent process is stopped with its thread"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the terminal session's PTY is stopped with its thread");
}

#[tokio::test]
async fn a_permission_card_restores_the_status_it_interrupted() {
    use oxplow_app::acp::host::AcpHost as _;
    use oxplow_domain::AgentStatusState;
    let (svc, root, _dir) = boot().await;
    let thread = seed(&svc, &root, ThreadStatus::Active).await;
    let host = ServicesAcpHost::new(&svc, Some(StreamId::new(1)), session_of(&svc, thread).await);
    let status = || async {
        let s = svc
            .agent_status_store
            .get(&thread, Some(session_of(&svc, thread).await))
            .await
            .unwrap()
            .unwrap();
        (s.state, s.detail)
    };
    // The agent already parked on the person (it asked them something).
    svc.hook_ingest
        .set_status(
            &thread,
            Some(session_of(&svc, thread).await),
            AgentStatusState::AwaitingUser,
            Some("Which DB?".into()),
        )
        .await
        .unwrap();
    host.awaiting_user(&thread, Some("Permission: Edit a.rs".into()))
        .await;
    assert_eq!(status().await.0, AgentStatusState::AwaitingUser);
    host.awaiting_user(&thread, None).await;
    assert_eq!(
        status().await,
        (AgentStatusState::AwaitingUser, Some("Which DB?".into())),
        "the earlier question survives the card"
    );

    // No turn open and nothing parked: answering leaves it idle, not running.
    svc.hook_ingest
        .set_status(
            &thread,
            Some(session_of(&svc, thread).await),
            AgentStatusState::Idle,
            None,
        )
        .await
        .unwrap();
    host.awaiting_user(&thread, Some("Permission: x".into()))
        .await;
    host.awaiting_user(&thread, None).await;
    assert_eq!(status().await.0, AgentStatusState::Idle);
}
