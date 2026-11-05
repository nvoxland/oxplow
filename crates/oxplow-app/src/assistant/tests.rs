//! The Assistant end to end over a Rig-backed provider (the shipped
//! `openai_compatible` kind) against a local stand-in for its API: the
//! rounds each request gets are scripted, and what each request carried is
//! read back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::{
    Access, Actor, Atomicity, CommandCall, CommandSpec, Confirm, Lifecycle, StreamId, ThreadId,
};
use serde_json::{json, Value};

use super::*;
use crate::ai_service::{ProviderConfig, Role, RoleBinding};
use crate::chat::model::ToolStatus;
use crate::chat::transcript::ItemBody;
use crate::commands::{Command, Handler, HandlerOutput, TxCtx};

const READ: &str = "oxplow.probe.read";
const WRITE: &str = "oxplow.probe.write";

fn chunk(delta: Value, finish: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({ "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
    )
}

fn done() -> Vec<String> {
    vec![
        format!(
            "data: {}\n\n",
            json!({ "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                    "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 } })
        ),
        "data: [DONE]\n\n".into(),
    ]
}

/// A round that answers `text`.
fn says(text: &str) -> Vec<String> {
    let mut out = vec![chunk(
        json!({ "role": "assistant", "content": text }),
        json!("stop"),
    )];
    out.extend(done());
    out
}

/// A round that calls `tool` with `args`.
fn calls(tool: &str, args: Value) -> Vec<String> {
    let mut out = vec![
        chunk(json!({ "role": "assistant", "content": "" }), Value::Null),
        chunk(
            json!({ "tool_calls": [{ "index": 0, "id": "call-1", "type": "function",
                "function": { "name": tool, "arguments": args.to_string() } }] }),
            Value::Null,
        ),
        chunk(json!({}), json!("tool_calls")),
    ];
    out.extend(done());
    out
}

struct Fx {
    _dir: tempfile::TempDir,
    svc: Arc<crate::Services>,
    seen: oxplow_ai_fake::Seen,
    stream: StreamId,
    thread: ThreadId,
}

impl Fx {
    async fn open(&self) -> String {
        self.svc
            .assistant
            .open(
                &self.svc,
                self.stream,
                Some(self.thread),
                Some(AssistantContext::Page {
                    uri: "oxplow:oxplow/wiki?slug=home".into(),
                    label: Some("Home".into()),
                }),
            )
            .await
            .unwrap()
            .id
    }

    async fn prompt(&self, id: &str, text: &str) {
        self.svc
            .assistant
            .prompt(&self.svc, id, text)
            .await
            .unwrap();
    }

    fn snapshot(&self, id: &str) -> AssistantSnapshot {
        self.svc.assistant.snapshot(id, 0).unwrap()
    }

    /// The snapshot once `pred` holds of it (30 s at most: a loaded machine).
    async fn until(
        &self,
        id: &str,
        pred: impl Fn(&AssistantSnapshot) -> bool,
    ) -> AssistantSnapshot {
        let started = Instant::now();
        loop {
            let snap = self.snapshot(id);
            if pred(&snap) {
                return snap;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "timed out: {snap:#?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn idle(&self, id: &str) -> AssistantSnapshot {
        self.until(id, |s| s.status == AssistantStatus::Idle).await
    }

    /// What request `n` sent.
    fn request(&self, n: usize) -> Value {
        self.seen.lock().unwrap()[n].2.clone()
    }

    async fn probe(&self) -> Option<String> {
        self.svc
            .db
            .read(|tx| {
                Ok(tx
                    .query_row("SELECT v FROM probe WHERE k = 'x'", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .ok())
            })
            .await
            .unwrap()
    }
}

fn probe_spec(id: &str, access: Access) -> CommandSpec {
    CommandSpec {
        output_schema: None,
        id: id.into(),
        summary: format!("Probe ({id})."),
        input_schema: json!({
            "type": "object",
            "properties": { "v": { "type": "string" } },
            "additionalProperties": false
        }),
        job: false,
        confirm: Confirm::Never,
        confirm_says: None,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        access,
        needs: Vec::new(),
        ui: None,
        op: None,
        unrecorded: Vec::new(),
    }
}

/// A read that answers 42, and an undoable write of `probe.x`.
fn register_probes(svc: &crate::Services) {
    let read = Handler::Tx(Arc::new(|_: &TxCtx<'_>, _| {
        Ok(HandlerOutput {
            result: json!({ "answer": 42 }),
            ..HandlerOutput::default()
        })
    }));
    svc.commands
        .register(Command::new(probe_spec(READ, Access::Read), read).unwrap())
        .unwrap();
    let write = Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let sql = |e: rusqlite::Error| oxplow_domain::CommandError::Failed {
            message: e.to_string(),
        };
        ctx.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS probe (k TEXT PRIMARY KEY, v TEXT)",
                [],
            )
            .map_err(sql)?;
        let before: Option<String> = ctx
            .conn
            .query_row("SELECT v FROM probe WHERE k = 'x'", [], |r| r.get(0))
            .ok();
        let v = input["v"].as_str().unwrap_or_default().to_string();
        ctx.conn
            .execute(
                "INSERT INTO probe (k, v) VALUES ('x', ?1) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [&v],
            )
            .map_err(sql)?;
        Ok(HandlerOutput {
            result: json!({ "v": v }),
            inverse: Some(CommandCall {
                name: WRITE.into(),
                input: json!({ "v": before.unwrap_or_default() }),
            }),
            ..HandlerOutput::default()
        })
    }));
    svc.commands
        .register(Command::new(probe_spec(WRITE, Access::Write), write).unwrap())
        .unwrap();
}

/// Services whose `assistant` role is bound (when `bound`) to a model
/// answering `rounds` in turn, each held open for its duration.
async fn fixture_held(rounds: Vec<(Vec<String>, Duration)>, bound: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let (base, seen) = oxplow_ai_fake::mock_sse_rounds_held("/chat/completions", rounds).await;
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    crate::test_fixtures::activate(&svc).await;
    svc.ai
        .save_provider(
            ProviderConfig {
                id: "a".into(),
                kind: "openai_compatible".into(),
                base_url: Some(base),
            },
            Some("sk-1".into()),
        )
        .unwrap();
    if bound {
        svc.ai
            .set_role(&Role::ASSISTANT, Some(RoleBinding::new("a", "m")))
            .unwrap();
    }
    register_probes(&svc);
    let stream = svc.streams.ensure_primary().await.unwrap();
    let thread = crate::test_fixtures::new_thread(&svc, stream.id, "Here").await;
    Fx {
        _dir: dir,
        svc,
        seen,
        stream: stream.id,
        thread: thread.id,
    }
}

async fn fixture(rounds: Vec<Vec<String>>) -> Fx {
    fixture_held(
        rounds.into_iter().map(|r| (r, Duration::ZERO)).collect(),
        true,
    )
    .await
}

fn tools_offered(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn tool_call(snap: &AssistantSnapshot) -> crate::chat::model::ToolCall {
    snap.items
        .iter()
        .find_map(|i| match &i.body {
            ItemBody::Tool { call } => Some(call.clone()),
            _ => None,
        })
        .expect("a tool call")
}

fn last_answer(snap: &AssistantSnapshot) -> String {
    snap.items
        .iter()
        .rev()
        .find_map(|i| match &i.body {
            ItemBody::Agent { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// A read runs as soon as the model calls it: no card, its result goes
/// back to the model. With changes off, no write tool is offered.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_tool_runs_unasked() {
    let fx = fixture(vec![
        calls("oxplow_probe_read", json!({})),
        says("It is 42."),
    ])
    .await;
    let id = fx.open().await;
    fx.prompt(&id, "What is the answer?").await;
    let snap = fx.idle(&id).await;
    let call = tool_call(&snap);
    assert_eq!(call.status, ToolStatus::Completed, "{snap:#?}");
    assert_eq!(call.name.as_deref(), Some(READ));
    assert_eq!(call.raw_output, Some(json!({ "answer": 42 })));
    assert_eq!(call.audit_id, None, "a read isn't audited");
    assert!(
        !snap
            .items
            .iter()
            .any(|i| matches!(i.body, ItemBody::Permission { .. })),
        "{snap:#?}"
    );
    assert_eq!(last_answer(&snap), "It is 42.");
    let offered = tools_offered(&fx.request(0));
    assert!(
        offered.contains(&"oxplow_probe_read".to_string()),
        "{offered:?}"
    );
    assert!(
        !offered.contains(&"oxplow_probe_write".to_string()),
        "{offered:?}"
    );
    assert!(
        !offered.contains(&tools::RUN_COMMAND.to_string()),
        "no change is offered with changes off: {offered:?}"
    );
    assert!(fx.request(1).to_string().contains("42"));
}

/// With changes allowed, a write waits on its card; allowed, it runs as
/// the person through the session (audited `via` it) and undoes by its
/// audit row.
#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_change_runs_audited_and_undoes() {
    let fx = fixture(vec![
        calls(
            tools::RUN_COMMAND,
            json!({ "command": WRITE, "input": { "v": "new" } }),
        ),
        says("Done."),
    ])
    .await;
    let id = fx.open().await;
    fx.svc
        .assistant
        .set(&fx.svc, &id, None, Some(true))
        .await
        .unwrap();
    fx.prompt(&id, "Write it.").await;
    let snap = fx
        .until(&id, |s| {
            s.items
                .iter()
                .any(|i| matches!(i.body, ItemBody::Permission { .. }))
        })
        .await;
    assert_eq!(snap.status, AssistantStatus::Running);
    assert_eq!(fx.probe().await, None, "nothing runs before the answer");
    let ask = snap
        .items
        .iter()
        .find_map(|i| match &i.body {
            ItemBody::Permission { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .unwrap();
    fx.svc.assistant.answer(&id, &ask, true).unwrap();
    let snap = fx.idle(&id).await;
    let call = tool_call(&snap);
    assert_eq!(call.status, ToolStatus::Completed, "{snap:#?}");
    assert_eq!(
        call.name.as_deref(),
        Some(WRITE),
        "the card names the command"
    );
    assert_eq!(fx.probe().await.as_deref(), Some("new"));
    let offered = tools_offered(&fx.request(0));
    assert!(
        offered.contains(&tools::RUN_COMMAND.to_string()),
        "{offered:?}"
    );
    assert!(
        offered.contains(&tools::FIND_COMMAND.to_string()),
        "{offered:?}"
    );
    assert!(
        !offered.contains(&"oxplow_probe_write".to_string()),
        "a change runs through run_command, not a tool of its own: {offered:?}"
    );
    let audit = call.audit_id.expect("the run's audit row");
    let row = fx
        .svc
        .commands
        .audit_store()
        .get(audit)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.command, WRITE);
    assert_eq!(
        row.actor_kind,
        oxplow_domain::events::schema::ActorKind::Human
    );
    assert_eq!(row.via.as_deref(), Some(format!("assistant:{id}").as_str()));
    fx.svc
        .commands
        .undo(&Actor::Human, audit, false)
        .await
        .unwrap();
    assert_eq!(fx.probe().await.as_deref(), Some(""));
    assert_eq!(last_answer(&snap), "Done.");
}

/// Declined, the call is skipped and the model is told so.
#[tokio::test(flavor = "multi_thread")]
async fn a_declined_change_is_skipped() {
    let fx = fixture(vec![
        calls(
            tools::RUN_COMMAND,
            json!({ "command": WRITE, "input": { "v": "new" } }),
        ),
        says("Okay, I won't."),
    ])
    .await;
    let id = fx.open().await;
    fx.svc
        .assistant
        .set(&fx.svc, &id, None, Some(true))
        .await
        .unwrap();
    fx.prompt(&id, "Write it.").await;
    let snap = fx
        .until(&id, |s| {
            s.items
                .iter()
                .any(|i| matches!(i.body, ItemBody::Permission { .. }))
        })
        .await;
    let ask = snap
        .items
        .iter()
        .find_map(|i| match &i.body {
            ItemBody::Permission { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .unwrap();
    fx.svc.assistant.answer(&id, &ask, false).unwrap();
    let snap = fx.idle(&id).await;
    assert_eq!(tool_call(&snap).status, ToolStatus::Failed);
    assert_eq!(fx.probe().await, None);
    assert!(
        fx.request(1).to_string().contains(session::DECLINED),
        "{}",
        fx.request(1)
    );
    assert_eq!(last_answer(&snap), "Okay, I won't.");
    assert!(
        fx.svc.assistant.answer(&id, &ask, true).is_err(),
        "answered once"
    );
}

/// Stopping a turn ends it at once; the next prompt runs.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_mid_turn_ends_promptly_and_the_next_prompt_works() {
    let thinking = vec![chunk(
        json!({ "role": "assistant", "content": "Thinking" }),
        Value::Null,
    )];
    let fx = fixture_held(
        vec![
            (thinking, Duration::from_secs(120)),
            (says("Back."), Duration::ZERO),
        ],
        true,
    )
    .await;
    let id = fx.open().await;
    fx.prompt(&id, "Take your time.").await;
    fx.until(&id, |s| last_answer(s) == "Thinking").await;
    let err = fx.svc.assistant.prompt(&fx.svc, &id, "again").await;
    assert_eq!(err, Err(AssistantError::TurnInFlight));
    let stopped = Instant::now();
    fx.svc.assistant.cancel(&id).unwrap();
    fx.idle(&id).await;
    assert!(stopped.elapsed() < Duration::from_secs(5));
    fx.prompt(&id, "Are you there?").await;
    let snap = fx.until(&id, |s| last_answer(s) == "Back.").await;
    assert_eq!(snap.status, AssistantStatus::Idle);
}

/// A second prompt carries the first exchange.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_prompt_carries_the_first_exchange() {
    let fx = fixture(vec![says("Hi Nathan."), says("Your name is Nathan.")]).await;
    let id = fx.open().await;
    fx.prompt(&id, "My name is Nathan.").await;
    fx.idle(&id).await;
    fx.prompt(&id, "What is my name?").await;
    let snap = fx
        .until(&id, |s| last_answer(s) == "Your name is Nathan.")
        .await;
    assert_eq!(snap.status, AssistantStatus::Idle);
    let second = fx.request(1)["messages"].to_string();
    assert!(second.contains("My name is Nathan."), "{second}");
    assert!(second.contains("Hi Nathan."), "{second}");
    // The system prompt names the page it was opened on.
    assert!(second.contains("oxplow:oxplow/wiki?slug=home"), "{second}");
    // Each model call is recorded, as the Assistant's.
    let calls = fx
        .svc
        .sql
        .query_sql(
            "SELECT role, caller, provider, model, input_tokens, output_tokens, ok FROM v_ai_call ORDER BY id",
            Vec::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(calls.rows.len(), 2, "{:?}", calls.rows);
    let row = serde_json::to_value(&calls.rows[0]).unwrap();
    let caller = format!("assistant:{id}");
    assert_eq!(
        row,
        json!(["assistant", caller, "a", "m", 10, 5, 1]),
        "{row}"
    );
}

/// With no model bound the session still opens, and says why it can't
/// answer.
#[tokio::test(flavor = "multi_thread")]
async fn no_model_bound_is_a_problem() {
    let fx = fixture_held(vec![(says("never"), Duration::ZERO)], false).await;
    let id = fx.open().await;
    let snap = fx.snapshot(&id);
    let problem = snap.problem.expect("a problem");
    assert_eq!(problem.kind, AssistantProblemKind::NoModel);
    assert!(
        problem.message.contains("Settings → AI"),
        "{}",
        problem.message
    );
    assert_eq!(snap.model, None);
    let err = fx.svc.assistant.prompt(&fx.svc, &id, "hello").await;
    assert!(matches!(err, Err(AssistantError::NoModel(_))), "{err:?}");
    assert!(fx.seen.lock().unwrap().is_empty(), "no model was called");
    // Picking a model clears it.
    fx.svc
        .assistant
        .set(
            &fx.svc,
            &id,
            Some(AssistantModelChoice::Model(AssistantModel {
                provider: "a".into(),
                model: "m".into(),
            })),
            None,
        )
        .await
        .unwrap();
    let snap = fx.snapshot(&id);
    assert_eq!(snap.problem, None);
    assert_eq!(snap.model.unwrap().model, "m");
}

/// `since` returns only what changed after it.
#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_since_returns_only_newer_items() {
    let fx = fixture(vec![says("One."), says("Two.")]).await;
    let id = fx.open().await;
    fx.prompt(&id, "first").await;
    let first = fx.idle(&id).await;
    let mark = first.seq;
    fx.prompt(&id, "second").await;
    fx.until(&id, |s| last_answer(s) == "Two.").await;
    let newer = fx.svc.assistant.snapshot(&id, mark).unwrap();
    assert!(newer.items.iter().all(|i| i.seq > mark), "{newer:#?}");
    let texts: Vec<String> = newer
        .items
        .iter()
        .filter_map(|i| match &i.body {
            ItemBody::User { text, .. } | ItemBody::Agent { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["second".to_string(), "Two.".to_string()]);
    assert!(fx.svc.assistant.snapshot("ast999", 0).is_err());
}

/// "Continue in session" hands a thread the condensed transcript: the
/// context's ref, the prompts, the answers and a line per tool call.
#[tokio::test(flavor = "multi_thread")]
async fn the_handoff_body_condenses_the_transcript() {
    let fx = fixture(vec![
        calls("oxplow_probe_read", json!({})),
        says("It is 42."),
    ])
    .await;
    let id = fx.open().await;
    fx.prompt(&id, "What is the answer?").await;
    fx.idle(&id).await;
    let other = crate::test_fixtures::new_thread(&fx.svc, fx.stream, "There").await;
    fx.svc
        .assistant
        .handoff(&fx.svc, &id, &other.id.to_string())
        .await
        .unwrap();
    let out = fx
        .svc
        .sql
        .query_sql("SELECT body FROM v_handoff", Vec::new(), None)
        .await
        .unwrap();
    let body = match &out.rows[0][0] {
        oxplow_db::SqlCell::Text(body) => body.clone(),
        other => panic!("{other:?}"),
    };
    for want in [
        "oxplow:oxplow/wiki?slug=home",
        "**Person:** What is the answer?",
        "**Assistant:** It is 42.",
        "`oxplow.probe.read`: done",
    ] {
        assert!(body.contains(want), "{want} in {body}");
    }
}

/// Closing a session ends it.
#[tokio::test(flavor = "multi_thread")]
async fn a_closed_session_is_gone() {
    let fx = fixture(vec![says("Hi.")]).await;
    let id = fx.open().await;
    fx.svc.assistant.close(&id).unwrap();
    assert!(matches!(
        fx.svc.assistant.snapshot(&id, 0),
        Err(AssistantError::NotFound(_))
    ));
}

/// With changes on, the model finds a change by what it does, with its
/// input schema; a read is refused through `run_command`.
#[tokio::test(flavor = "multi_thread")]
async fn find_command_lists_the_changes_and_run_command_runs_only_changes() {
    let fx = fixture(vec![]).await;
    let found = tools::find(&fx.svc.commands, &json!({ "query": "probe" }));
    let commands: Vec<&str> = found["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, vec![WRITE], "{found}");
    assert_eq!(found["commands"][0]["input_schema"]["type"], "object");
    let err = tools::changing_command(&fx.svc.commands, READ).unwrap_err();
    assert!(err.contains("oxplow_probe_read"), "{err}");
    assert!(tools::changing_command(&fx.svc.commands, "oxplow.nope").is_err());
}
