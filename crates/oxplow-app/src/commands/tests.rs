//! The command bus's own tests (`commands/mod.rs`).

use super::*;
use oxplow_domain::events::schema::{ActorKind, ConfigChanged, ConfigChangedV2};
use oxplow_domain::{Access, Confirm, Invokers, Lifecycle, ThreadId};
use serde_json::json;

fn bus() -> (Database, CommandBus) {
    let db = Database::in_memory();
    let log = SqliteEventLogStore::new(
        db.clone(),
        oxplow_domain::vocabulary::VocabularyHandle::core(),
    );
    let pump = Arc::new(EventPump::new(db.clone(), log.clone(), vec![]));
    let bus = CommandBus::new(db.clone(), log, Arc::new(AgentPolicy), pump);
    db.clone()
        .transaction(|tx| {
            // The table the kv commands write, and the threads the
            // tests' agents run in (a proposal names one; the bus
            // resolves an agent's stream from it).
            tx.execute_batch(
                "CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
                 INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x',
                           '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (7, 1, 'T', 'active', '2026-01-01', '2026-01-01'),
                          (8, 1, 'U', 'queued', '2026-01-01', '2026-01-01');",
            )
            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
        })
        .now_or_never_ok();
    (db, bus)
}

trait NowOrNever {
    fn now_or_never_ok(self);
}
impl<F: Future<Output = Result<(), oxplow_domain::DomainError>>> NowOrNever for F {
    fn now_or_never_ok(self) {
        let rt = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| rt.block_on(self)).unwrap();
    }
}

/// A lock blip inside a `Tx` handler (SQLITE_BUSY, a snapshot that
/// moved under a read-then-write) retries the run like any transaction,
/// instead of failing the caller; a persistent one surfaces as `Busy`.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_busy_tx_handler_is_retried_then_reported_as_busy() {
    let (_db, bus) = bus();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = attempts.clone();
    let flaky = Handler::Tx(Arc::new(move |_ctx: &TxCtx<'_>, input| {
        let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if input["v"] == "always" || n == 0 {
            return Err(CommandError::from(oxplow_domain::DomainError::Busy(
                "database is locked".into(),
            )));
        }
        Ok(HandlerOutput {
            result: json!({ "attempt": n }),
            inverse: None,
            events: Vec::new(),
            after_commit: None,
            unchanged: false,
        })
    }));
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.flaky", Invokers::ALL, Confirm::Never),
            flaky,
        )
        .unwrap(),
    )
    .unwrap();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.kv.flaky",
            json!({"k": "a", "v": "once"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        out.result["attempt"], 1,
        "the second attempt ran and committed"
    );
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.flaky",
            json!({"k": "a", "v": "always"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Busy { .. }), "{err:?}");
}

fn kv_spec(name: &str, invokers: Invokers, confirm: Confirm) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: "Set a key in the test table.".into(),
        input_schema: json!({
            "type": "object",
            "required": ["k", "v"],
            "properties": { "k": { "type": "string" }, "v": { "type": "string" } },
            "additionalProperties": false
        }),
        invokers,
        confirm,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        access: Access::Write,
        needs: Vec::new(),
        ui: None,
        op: None,
        unrecorded: Vec::new(),
    }
}

/// A `Tx` handler: writes `k = v`, returns the inverse (restore the
/// prior value) and a `config.changed` domain event.
fn kv_set() -> Handler {
    Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let conn = ctx.conn;
        let k = input["k"].as_str().unwrap_or_default().to_string();
        let v = input["v"].as_str().unwrap_or_default().to_string();
        if v == "boom" {
            return Err(CommandError::Failed {
                message: "refused value".into(),
            });
        }
        let before: Option<String> = conn
            .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| r.get(0))
            .ok();
        conn.execute(
            "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
            [&k, &v],
        )
        .map_err(|e| CommandError::Failed {
            message: e.to_string(),
        })?;
        if v == "half" {
            // A write happened, then the handler fails: it must roll back.
            return Err(CommandError::Failed {
                message: "failed after writing".into(),
            });
        }
        Ok(HandlerOutput {
            result: json!({ "k": k, "v": v }),
            inverse: Some(CommandCall {
                name: "oxplow.kv.set".into(),
                input: json!({ "k": k, "v": before.clone().unwrap_or_default() }),
            }),
            events: vec![Envelope::typed::<ConfigChanged>(
                "test",
                &ConfigChangedV2 {
                    key: k.clone(),
                    before: before.map(Value::String).unwrap_or(Value::Null),
                    after: Value::String(v),
                    layer: oxplow_domain::events::schema::ConfigLayer::Project,
                },
            )],
            after_commit: None,
            unchanged: false,
        })
    }))
}

async fn kv_value(db: &Database, k: &str) -> Option<String> {
    let k = k.to_string();
    db.read(move |tx| {
        Ok(tx
            .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| {
                r.get::<_, String>(0)
            })
            .ok())
    })
    .await
    .unwrap()
}

fn agent() -> Actor {
    Actor::Agent {
        session_id: None,
        thread_id: Some(ThreadId::new(7)),
        stream_id: None,
    }
}

/// A `Read` handler runs in a snapshot that is always rolled back, so
/// a write it makes (by mistake) never lands — it isn't audited.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_commands_write_never_lands() {
    let (db, bus) = bus();
    let mut spec = kv_spec("oxplow.kv.sneaky", Invokers::ALL, Confirm::Never);
    spec.access = Access::Read;
    bus.register(Command::new(spec, kv_set()).unwrap()).unwrap();
    bus.run(
        &agent(),
        "oxplow.kv.sneaky",
        json!({"k": "a", "v": "1"}),
        false,
    )
    .await
    .unwrap();
    assert_eq!(kv_value(&db, "a").await, None);
}

/// A `View` runs as a `Read` does: neither is recorded.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_command_is_not_recorded() {
    for access in [Access::View, Access::Read] {
        let (_db, bus) = bus();
        let mut spec = kv_spec("oxplow.kv.get", Invokers::ALL, Confirm::Never);
        spec.access = access;
        bus.register(
            Command::new(
                spec,
                Handler::Tx(Arc::new(|_ctx: &TxCtx<'_>, input| {
                    Ok(HandlerOutput {
                        result: input,
                        ..HandlerOutput::default()
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
        let out = bus
            .run(
                &agent(),
                "oxplow.kv.get",
                json!({"k": "a", "v": "1"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["k"], "a");
        assert_eq!((out.audit_id, out.event_id), (None, None), "{access:?}");
        assert!(bus.log.read_after(0, 10).await.unwrap().is_empty());
        assert!(bus.audit_store().list_recent(10).await.unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_thread_that_may_not_write_is_refused_writes_not_reads() {
    let (db, bus) = bus();
    let bus = bus.with_write_gate(Arc::new(|thread| {
        Box::pin(async move { thread != ThreadId::new(7) })
    }));
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &agent(),
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "a").await, None);
    // Another thread may; a person is never gated.
    let other = Actor::Agent {
        session_id: None,
        thread_id: Some(ThreadId::new(8)),
        stream_id: None,
    };
    bus.run(&other, "oxplow.kv.set", json!({"k": "a", "v": "1"}), false)
        .await
        .unwrap();
    bus.run(
        &Actor::Human,
        "oxplow.kv.set",
        json!({"k": "b", "v": "1"}),
        false,
    )
    .await
    .unwrap();
}

/// An effect is gated like the agent whose rights it carries: reacting to
/// an event on a thread that may not write, its writes are refused; on
/// one that may, or with no thread, they run.
#[tokio::test(flavor = "multi_thread")]
async fn an_effect_is_held_to_its_events_threads_writer_gate() {
    let (db, bus) = bus();
    let bus = bus.with_write_gate(Arc::new(|thread| {
        Box::pin(async move { thread != ThreadId::new(7) })
    }));
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let on = |thread: Option<i64>| Actor::Effect {
        effect: "acme/notify".into(),
        thread_id: thread.map(ThreadId::new),
        stream_id: None,
    };
    let err = bus
        .run(
            &on(Some(7)),
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "a").await, None);
    bus.run(
        &on(Some(8)),
        "oxplow.kv.set",
        json!({"k": "a", "v": "1"}),
        false,
    )
    .await
    .unwrap();
    bus.run(
        &on(None),
        "oxplow.kv.set",
        json!({"k": "b", "v": "1"}),
        false,
    )
    .await
    .unwrap();
}

/// A composite carries step 3's answer to its children: a thread that
/// may not write can't write through `oxplow.command.sequence` either.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_that_may_not_write_cant_write_through_a_composite() {
    let (db, bus) = bus();
    let bus = Arc::new(bus.with_write_gate(Arc::new(|thread| {
        Box::pin(async move { thread != ThreadId::new(7) })
    })));
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    // A `Record` composite isn't gated itself (it may only record), so
    // what refuses the run is its `Write` child seeing the answer.
    let mut spec = kv_spec("oxplow.kv.compose", Invokers::ALL, Confirm::Never);
    spec.access = Access::Record;
    spec.input_schema = json!({ "type": "object" });
    let parent = spec.clone();
    let weak = Arc::downgrade(&bus);
    bus.register(
        Command::new(
            spec,
            Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
                let calls: Vec<CommandCall> =
                    serde_json::from_value(input["calls"].clone()).unwrap();
                weak.upgrade().unwrap().run_nested(ctx, &parent, &calls)
            })),
        )
        .unwrap(),
    )
    .unwrap();
    let calls = json!({ "calls": [{ "name": "oxplow.kv.set", "input": { "k": "a", "v": "1" } }] });
    let err = bus
        .run(&agent(), "oxplow.kv.compose", calls.clone(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "a").await, None);
    // A thread that may write gets through the same composite.
    let other = Actor::Agent {
        session_id: None,
        thread_id: Some(ThreadId::new(8)),
        stream_id: None,
    };
    bus.run(&other, "oxplow.kv.compose", calls, false)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tx_command_writes_its_state_audit_and_events_together() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let out = bus
        .run(
            &agent(),
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.result, json!({"k": "a", "v": "1"}));
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    let audit = bus
        .audit_store()
        .get(out.audit_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.command, "oxplow.kv.set");
    assert_eq!(audit.thread_id, Some(ThreadId::new(7)));
    assert_eq!(audit.outcome, Outcome::Ok);
    assert_eq!(audit.event_id, out.event_id.clone());
    assert_eq!(
        audit.inverse.as_ref().unwrap().input,
        json!({"k": "a", "v": ""})
    );
    // command.executed then the handler's domain event, caused by it,
    // anchored to the actor's thread like the run itself.
    let events = bus.log.read_after(0, 10).await.unwrap();
    assert_eq!(events[1].envelope.anchors.thread_id, Some(ThreadId::new(7)));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].envelope.event_type, "command.executed");
    assert_eq!(
        events[0].envelope.payload["audit_id"],
        out.audit_id.unwrap()
    );
    assert_eq!(events[0].envelope.payload["actor_kind"], "agent");
    assert_eq!(events[0].envelope.source, "agent:thr7");
    assert_eq!(events[0].envelope.subject, vec!["command:oxplow.kv.set"]);
    assert_eq!(events[1].envelope.event_type, "config.changed");
    assert_eq!(events[1].envelope.cause, out.event_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_tx_handler_rolls_everything_back_and_is_audited_as_error() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": "half"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
    assert_eq!(
        kv_value(&db, "a").await,
        None,
        "the handler's write rolled back"
    );
    assert!(bus.log.read_after(0, 10).await.unwrap().is_empty());
    let recent = bus.audit_store().list_recent(5).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].outcome, Outcome::Error);
    assert!(recent[0]
        .error
        .as_deref()
        .unwrap()
        .contains("failed after writing"));
}

#[tokio::test(flavor = "multi_thread")]
async fn schema_rejection_names_the_field_and_is_audited_invalid() {
    let (_db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": 3}),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/v"),
        "{err:?}"
    );
    assert_eq!(
        bus.audit_store().list_recent(1).await.unwrap()[0].outcome,
        Outcome::Invalid
    );
    assert!(matches!(
        bus.run(&Actor::Human, "oxplow.kv.nope", json!({}), false)
            .await
            .unwrap_err(),
        CommandError::Unknown { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn invokers_and_agent_policy_deny_before_anything_runs() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::HUMAN_ONLY, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let lens = Actor::Lens {
        lens_id: "acme/x".into(),
        on_behalf_of: Box::new(Actor::Human),
    };
    let err = bus
        .run(&lens, "oxplow.kv.set", json!({"k": "a", "v": "1"}), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    let err = bus
        .run(
            &agent(),
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "a").await, None);
    let recent = bus.audit_store().list_recent(5).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert!(recent.iter().all(|r| r.outcome == Outcome::Denied));
    // `list` shows each actor only what it may run.
    assert!(bus.list(&lens).is_empty());
    assert_eq!(bus.list(&Actor::Human).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmation_is_a_persons_move_and_an_agent_never_gets_past_it() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Always),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    // A person: asked first, then confirmed.
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    let CommandError::NeedsConfirmation { preview } = err else {
        panic!("{err:?}");
    };
    assert_eq!(preview.command, "oxplow.kv.set");
    assert!(!preview.destructive);
    assert_eq!(kv_value(&db, "a").await, None);
    assert!(
        bus.audit_store().list_recent(5).await.unwrap().is_empty(),
        "nothing audited"
    );
    bus.run(
        &Actor::Human,
        "oxplow.kv.set",
        json!({"k": "a", "v": "1"}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    // An agent: `confirmed` is ignored; nothing is written — the run
    // waits as a proposal for a person.
    let err = bus
        .run(&agent(), "oxplow.kv.set", json!({"k": "b", "v": "2"}), true)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "b").await, None);
}

/// A `Tx` handler that writes `k = v` and says it changed nothing —
/// with `events` when `v == "with-events"`; `inverse` names
/// `oxplow.kv.noop`.
fn kv_unchanged() -> Handler {
    Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let k = input["k"].as_str().unwrap_or_default().to_string();
        let v = input["v"].as_str().unwrap_or_default().to_string();
        ctx.conn
            .execute(
                "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [&k, &v],
            )
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
        let events = if v == "with-events" {
            vec![Envelope::typed::<ConfigChanged>(
                "test",
                &ConfigChangedV2 {
                    key: k.clone(),
                    before: Value::Null,
                    after: Value::String(v.clone()),
                    layer: oxplow_domain::events::schema::ConfigLayer::Project,
                },
            )]
        } else {
            Vec::new()
        };
        Ok(HandlerOutput {
            result: json!({ "k": k }),
            inverse: None,
            events,
            after_commit: None,
            unchanged: true,
        })
    }))
}

/// A `Tx` handler that writes `k = v` and is undone by `oxplow.kv.noop`.
fn kv_undone_by_noop() -> Handler {
    Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        let k = input["k"].as_str().unwrap_or_default().to_string();
        let v = input["v"].as_str().unwrap_or_default().to_string();
        ctx.conn
            .execute("INSERT INTO kv (k, v) VALUES (?1, ?2)", [&k, &v])
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
        Ok(HandlerOutput {
            result: json!({ "k": k }),
            inverse: Some(CommandCall {
                name: "oxplow.kv.noop".into(),
                input: json!({ "k": format!("{k}-undo"), "v": "x" }),
            }),
            ..HandlerOutput::default()
        })
    }))
}

/// tsk901: a call that says it changed nothing keeps nothing — what it
/// wrote is rolled back, so it can't land unaudited — and one that
/// returns events (a change) while saying so is refused.
#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_call_keeps_nothing_and_cant_carry_events() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.noop", Invokers::ALL, Confirm::Never),
            kv_unchanged(),
        )
        .unwrap(),
    )
    .unwrap();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.kv.noop",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.audit_id, None);
    assert_eq!(kv_value(&db, "a").await, None, "its write rolled back");
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.noop",
            json!({"k": "b", "v": "with-events"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("changed nothing"), "{err}");
    assert_eq!(kv_value(&db, "b").await, None);
    // The refused call is audited as the failure it is; the unchanged
    // one left nothing.
    let rows = audits(&db).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].outcome,
        oxplow_domain::events::schema::CommandOutcome::Error
    );
}

/// tsk901: an undo, or a person's approval, whose run changes nothing
/// is still recorded — its row has to be marked.
#[tokio::test(flavor = "multi_thread")]
async fn an_undo_or_approval_that_changes_nothing_is_still_recorded() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.noop", Invokers::ALL, Confirm::Always),
            kv_unchanged(),
        )
        .unwrap(),
    )
    .unwrap();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.mark", Invokers::ALL, Confirm::Never),
            kv_undone_by_noop(),
        )
        .unwrap(),
    )
    .unwrap();
    let marked = bus
        .run(
            &Actor::Human,
            "oxplow.kv.mark",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap();
    let undo = bus
        .undo(&Actor::Human, marked.audit_id.unwrap(), true)
        .await
        .unwrap();
    assert!(undo.audit_id.is_some(), "the undo is recorded");
    let row = bus
        .audit_store()
        .get(marked.audit_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.undone_by, undo.audit_id);

    let err = bus
        .run(
            &agent(),
            "oxplow.kv.noop",
            json!({"k": "c", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    let rows = pending(&db).await;
    let approved = bus.approve(&Actor::Human, rows[0].id).await.unwrap();
    assert!(approved.audit_id.is_some(), "the approval is recorded");
    let p = proposal(&db, rows[0].id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_applies_the_inverse_and_marks_the_row_once() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    bus.run(
        &Actor::Human,
        "oxplow.kv.set",
        json!({"k": "a", "v": "1"}),
        false,
    )
    .await
    .unwrap();
    let second = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": "2"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("2"));
    let undo = bus
        .undo(&Actor::Human, second.audit_id.unwrap(), false)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    let row = bus
        .audit_store()
        .get(second.audit_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.undone_by, undo.audit_id);
    let err = bus
        .undo(&Actor::Human, second.audit_id.unwrap(), false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already undone"), "{err}");
    assert!(bus.undo(&Actor::Human, 9999, false).await.is_err());
}

/// Two undos of one row race: exactly one applies the inverse; the
/// other is refused without running it (an inverse applied twice is a
/// second, unasked-for change).
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_undos_apply_the_inverse_once() {
    let (db, bus) = bus();
    let bus = Arc::new(bus);
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    bus.run(
        &Actor::Human,
        "oxplow.kv.set",
        json!({"k": "a", "v": "1"}),
        false,
    )
    .await
    .unwrap();
    let second = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": "2"}),
            false,
        )
        .await
        .unwrap()
        .audit_id
        .unwrap();
    let (b1, b2) = (bus.clone(), bus.clone());
    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { b1.undo(&Actor::Human, second, false).await }),
        tokio::spawn(async move { b2.undo(&Actor::Human, second, false).await }),
    );
    let results = [r1.unwrap(), r2.unwrap()];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    let ok_runs = bus
        .audit_store()
        .list_recent(20)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.outcome == Outcome::Ok)
        .count();
    assert_eq!(ok_runs, 3, "two sets and one undo");
}

/// A lens acting for an agent is held to the agent rules: it can't
/// confirm (its run is proposed), and the agent policy applies.
#[tokio::test(flavor = "multi_thread")]
async fn a_lens_acting_for_an_agent_is_treated_as_the_agent() {
    let (_db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Always),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let lens = Actor::Lens {
        lens_id: "acme/x".into(),
        on_behalf_of: Box::new(agent()),
    };
    let err = bus
        .run(&lens, "oxplow.kv.set", json!({"k": "a", "v": "1"}), true)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    let for_person = Actor::Lens {
        lens_id: "acme/x".into(),
        on_behalf_of: Box::new(Actor::Human),
    };
    bus.run(
        &for_person,
        "oxplow.kv.set",
        json!({"k": "a", "v": "1"}),
        true,
    )
    .await
    .unwrap();
}

/// An `External` handler's effects have happened by the time the bus
/// records them; if recording then fails, the run still happened —
/// it's reported as done (unrecorded), not as an error.
#[tokio::test(flavor = "multi_thread")]
async fn an_external_run_whose_record_fails_still_reports_success() {
    let (db, bus) = bus();
    let mut spec = kv_spec("oxplow.kv.effort", Invokers::ALL, Confirm::Never);
    spec.atomicity = Atomicity::External;
    let writes = db.clone();
    bus.register(
        Command::new(
            spec,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let db = writes.clone();
                Box::pin(async move {
                    db.transaction(|tx| {
                        tx.execute("INSERT INTO kv (k, v) VALUES ('e', '1')", [])
                            .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
                        Ok(())
                    })
                    .await
                    .map_err(CommandError::from)?;
                    Ok(HandlerOutput {
                        result: input,
                        // An event the log refuses (unregistered type):
                        // recording fails after the write committed.
                        events: vec![Envelope::new("nope.unknown", 1, "test", json!({})).unwrap()],
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.kv.effort",
            json!({"k": "e", "v": "1"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.audit_id, None);
    assert_eq!(kv_value(&db, "e").await.as_deref(), Some("1"));
    let rows = bus.audit_store().list_recent(10).await.unwrap();
    assert!(rows.iter().all(|r| r.outcome != Outcome::Error), "{rows:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_checks_names_atomicity_and_collisions() {
    let (_db, bus) = bus();
    let mut wrong = kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never);
    wrong.atomicity = Atomicity::External;
    assert!(Command::new(wrong, kv_set()).is_err());
    assert!(Command::new(kv_spec("set", Invokers::ALL, Confirm::Never), kv_set()).is_err());
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(bus
        .register(
            Command::new(
                kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
                kv_set()
            )
            .unwrap()
        )
        .is_err());
    assert!(bus.external_commands().is_empty());
    // A read (or a view) is never asked about: nothing would resolve its
    // proposal.
    for access in [Access::View, Access::Read] {
        let mut asking_read = kv_spec("oxplow.kv.peek", Invokers::ALL, Confirm::Always);
        asking_read.access = access;
        let err = Command::new(asking_read, kv_set()).err().unwrap();
        assert!(err.to_string().contains("only reads"), "{err}");
        let mut read = kv_spec("oxplow.kv.peek", Invokers::ALL, Confirm::Never);
        read.access = access;
        let err = Command::new(read, kv_set())
            .unwrap()
            .with_confirm_for(Arc::new(|_| Confirm::Always))
            .err()
            .unwrap();
        assert!(err.to_string().contains("only reads"), "{err}");
    }
}

/// One key may run several commands told apart by their `when` (VS
/// Code's way), but two on the same key under the same `when` always
/// collide: the second is refused where it registers. A `when` that
/// doesn't check out is refused too.
#[tokio::test(flavor = "multi_thread")]
async fn a_shortcut_is_held_once_per_when() {
    let (_db, bus) = bus();
    let keyed = |id: &str, when: Option<&str>| {
        let mut spec = kv_spec(id, Invokers::ALL, Confirm::Never);
        spec.ui = Some(oxplow_domain::CommandUi {
            label: id.into(),
            shortcut: Some("Ctrl/Cmd+K".into()),
            when: when.map(str::to_string),
            ..Default::default()
        });
        Command::new(spec, kv_set())
    };
    bus.register(keyed("oxplow.kv.a", Some("fileShown")).unwrap())
        .unwrap();
    bus.register(keyed("oxplow.kv.b", Some("!fileShown")).unwrap())
        .unwrap();
    let err = bus
        .register(keyed("oxplow.kv.c", Some("fileShown")).unwrap())
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { field: Some(f), message }
            if f == "/ui/shortcut" && message.contains("oxplow.kv.a")),
        "{err:?}"
    );
    let err = keyed("oxplow.kv.d", Some("fileShwn")).err().unwrap();
    assert!(
        matches!(&err, CommandError::Invalid { field: Some(f), message }
            if f == "/ui/when" && message.contains("isn't a context key")),
        "{err:?}"
    );
}

/// An agent's undo or approval that needs a person is refused, not
/// proposed as a plain call that would lose what it undoes.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_undo_that_asks_is_denied_not_proposed() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Always),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let done = bus
        .run(
            &Actor::Human,
            "oxplow.kv.set",
            json!({"k": "a", "v": "1"}),
            true,
        )
        .await
        .unwrap();
    let err = bus
        .undo(&agent(), done.audit_id.unwrap(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err}");
    assert!(pending(&db).await.is_empty());
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
}

/// A namespace is registered whole or not at all, by one owner; core
/// commands' namespaces are oxplow's.
#[tokio::test(flavor = "multi_thread")]
async fn a_namespace_is_one_owners_and_oxplow_is_oxplows() {
    let (_db, bus) = bus();
    let cmd =
        |name: &str| Command::new(kv_spec(name, Invokers::ALL, Confirm::Never), kv_set()).unwrap();
    bus.register(cmd("oxplow.kv.set")).unwrap();
    assert_eq!(bus.namespace_owner("oxplow").as_deref(), Some("oxplow"));
    assert_eq!(bus.source_of("oxplow.kv.set").as_deref(), Some(CORE_SOURCE));
    assert_eq!(bus.namespace_owner("ext"), None);

    // An id that isn't `<namespace>.<area>.<verb>` is no command at
    // all; one outside the namespace: nothing is registered.
    let err = Command::new(kv_spec("ext.a", Invokers::ALL, Confirm::Never), kv_set())
        .err()
        .expect("a two-part id is refused");
    assert!(
        err.to_string().contains("<namespace>.<area>.<verb>"),
        "{err}"
    );
    let err = bus
        .register_namespace(
            "ext",
            "extension:x",
            vec![cmd("ext.thing.a"), cmd("other.thing.b")],
        )
        .unwrap_err();
    assert!(err.to_string().contains("other.thing.b"), "{err}");
    assert_eq!(bus.namespace_owner("ext"), None);
    assert!(bus.input_schema("ext.thing.a").is_none());

    bus.register_namespace(
        "ext",
        "extension:x",
        vec![cmd("ext.thing.a"), cmd("ext.thing.b")],
    )
    .unwrap();
    assert_eq!(bus.namespace_owner("ext").as_deref(), Some("extension:x"));
    // Another source's namespace is taken.
    let err = bus
        .register_namespace("ext", "provider:ext", vec![cmd("ext.thing.c")])
        .unwrap_err();
    assert!(err.to_string().contains("extension:x"), "{err}");

    // `oxplow` is reserved: an extension that doesn't ship with oxplow
    // is refused; one that does shares it, ids checked one by one.
    let err = bus
        .register_namespace("oxplow", "extension:acme", vec![cmd("oxplow.kv.other")])
        .unwrap_err();
    assert!(err.to_string().contains("reserved"), "{err}");
    let err = bus
        .register_namespace(
            "oxplow",
            "extension:oxplow-bundled",
            vec![cmd("oxplow.kv.set")],
        )
        .unwrap_err();
    assert!(err.to_string().contains("already registered"), "{err}");
    bus.register_namespace(
        "oxplow",
        "extension:oxplow-bundled",
        vec![cmd("oxplow.review.accept")],
    )
    .unwrap();
    assert_eq!(bus.namespace_owner("oxplow").as_deref(), Some("oxplow"));

    // Unregistering a source takes only its commands.
    assert_eq!(
        bus.unregister_source("extension:oxplow-bundled"),
        vec!["oxplow.review.accept".to_string()]
    );
    assert!(bus.input_schema("oxplow.kv.set").is_some());
    assert_eq!(bus.unregister_source("extension:x").len(), 2);
    assert_eq!(bus.namespace_owner("ext"), None);
}

// ---- P6b.A1: the bus composes ----

/// A handler that only writes when the run was confirmed: it sees
/// step 4's answer in `ctx.confirmed`. A refusal it raises itself is
/// a confirmation, not an error — rolled back, nothing audited.
fn kv_set_if_confirmed() -> Handler {
    Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
        if !ctx.confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: "oxplow.kv.careful".into(),
                    summary: "asks".into(),
                    input: input.clone(),
                    destructive: false,
                }),
            });
        }
        ctx.conn
            .execute(
                "INSERT INTO kv (k, v) VALUES (?1, ?2)",
                [input["k"].as_str().unwrap(), input["v"].as_str().unwrap()],
            )
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
        Ok(HandlerOutput::default())
    }))
}

async fn audits(db: &Database) -> Vec<CommandAudit> {
    SqliteCommandAuditStore::new(db.clone())
        .list_recent(50)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tx_handler_sees_whether_the_run_was_confirmed() {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.careful", Invokers::ALL, Confirm::Never),
            kv_set_if_confirmed(),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.kv.careful",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, CommandError::NeedsConfirmation { .. }),
        "{err:?}"
    );
    assert_eq!(kv_value(&db, "a").await, None);
    assert!(
        audits(&db).await.is_empty(),
        "a confirmation is not an error"
    );
    bus.run(
        &Actor::Human,
        "oxplow.kv.careful",
        json!({"k": "a", "v": "1"}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
}

/// `oxplow.command.sequence` composes Tx commands in one run: each child's
/// own invokers, policy and confirmation apply; the children share the
/// parent's transaction and audit row; the inverse is the children's
/// inverses, reversed.
fn composing_bus() -> (Database, Arc<CommandBus>) {
    let (db, bus) = bus();
    composing_bus_on(db, bus)
}

/// [`composing_bus`]'s commands on `bus` (one with capabilities, say).
fn composing_bus_on(db: Database, bus: CommandBus) -> (Database, Arc<CommandBus>) {
    let bus = Arc::new(bus);
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.secret", Invokers::HUMAN_ONLY, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.danger", Invokers::ALL, Confirm::Destructive),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut plain = kv_spec("oxplow.kv.plain", Invokers::ALL, Confirm::Never);
    plain.undoable = false;
    bus.register(Command::new(plain, kv_set()).unwrap())
        .unwrap();
    // A system outside the bus's transaction: it writes `kv` in its own
    // transaction, and refuses the value `fail`.
    let mut ext = kv_spec("oxplow.kv.external", Invokers::ALL, Confirm::Never);
    ext.atomicity = Atomicity::External;
    let outside = db.clone();
    bus.register(
        Command::new(
            ext,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let db = outside.clone();
                Box::pin(async move {
                    let k = input["k"].as_str().unwrap_or_default().to_string();
                    let v = input["v"].as_str().unwrap_or_default().to_string();
                    if v == "fail" {
                        return Err(CommandError::Failed {
                            message: "the system refused it".into(),
                        });
                    }
                    db.transaction(move |tx| {
                        tx.execute(
                            "INSERT INTO kv (k, v) VALUES (?1, ?2)
                             ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                            [&k, &v],
                        )
                        .map(|_| ())
                        .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                    })
                    .await
                    .map_err(CommandError::from)?;
                    Ok(HandlerOutput {
                        result: input,
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    bus.register(super::compose::sequence_command()).unwrap();
    (db, bus)
}

fn calls(items: &[(&str, &str, &str)]) -> Value {
    json!({ "calls": items.iter().map(|(n, k, v)| json!({ "name": n, "input": { "k": k, "v": v } })).collect::<Vec<_>>() })
}

/// Step 0 holds for a composed call as for a direct one: a child
/// that needs what isn't active (a work list, say) is
/// refused at its place in the calls — whether the composite runs in
/// one transaction or as steps — and nothing lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_composed_call_needs_what_a_direct_one_does() {
    let (db, bus) = bus();
    // Nothing is declared, so no capability is active.
    let registry = Arc::new(crate::capabilities::CapabilityRegistry::new(
        Vec::new(),
        oxplow_domain::vocabulary::VocabularyHandle::core(),
    ));
    let config = Arc::new(std::sync::RwLock::new(
        oxplow_config::load_project_config(tempfile::tempdir().unwrap().path()).unwrap(),
    ));
    let (db, bus) = composing_bus_on(db, bus.with_capabilities(registry, config));
    let mut needy = kv_spec("oxplow.kv.needy", Invokers::ALL, Confirm::Never);
    needy.needs = vec!["work_items".into()];
    bus.register(Command::new(needy, kv_set()).unwrap())
        .unwrap();

    let direct = bus
        .run(
            &Actor::Human,
            "oxplow.kv.needy",
            json!({ "k": "n", "v": "1" }),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(direct, CommandError::Invalid { .. }), "{direct:?}");

    for (first, why) in [
        ("oxplow.kv.set", "in one transaction"),
        ("oxplow.kv.external", "as steps"),
    ] {
        let err = bus
            .run(
                &Actor::Human,
                "oxplow.command.sequence",
                calls(&[(first, "a", "1"), ("oxplow.kv.needy", "n", "1")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/calls/1/name" && message.contains("`oxplow.kv.needy`: Needs: Work list")),
            "{why}: {err:?}"
        );
        assert_eq!(kv_value(&db, "a").await, None, "{why}: nothing landed");
        assert_eq!(kv_value(&db, "n").await, None, "{why}");
    }
}

/// A run's actor is resolved once: an agent whose transport carried its
/// thread but not its stream runs with its thread's stream — through a
/// lens too — so every handler's "the caller's stream" is the same
/// answer; an agent claiming a thread that doesn't exist is refused.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_stream_is_its_threads() {
    let (_db, bus) = bus();
    let mut spec = kv_spec("oxplow.kv.whoami", Invokers::ALL, Confirm::Never);
    spec.access = Access::Read;
    spec.input_schema = json!({ "type": "object" });
    bus.register(
        Command::new(
            spec,
            Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, _| {
                Ok(HandlerOutput {
                    result: json!(ctx.actor.stream_id().map(|s| s.to_string())),
                    ..HandlerOutput::default()
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    let agent = |thread| Actor::Agent {
        session_id: None,
        thread_id: Some(ThreadId::new(thread)),
        stream_id: None,
    };
    let out = bus
        .run(&agent(7), "oxplow.kv.whoami", json!({}), false)
        .await
        .unwrap();
    assert_eq!(out.result, json!("str1"));
    let through_a_lens = Actor::Lens {
        lens_id: "acme/x".into(),
        on_behalf_of: Box::new(agent(7)),
    };
    let out = bus
        .run(&through_a_lens, "oxplow.kv.whoami", json!({}), false)
        .await
        .unwrap();
    assert_eq!(out.result, json!("str1"));
    let err = bus
        .run(&agent(99), "oxplow.kv.whoami", json!({}), false)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Denied { reason } if reason.contains("thr99")),
        "{err:?}"
    );
}

/// A composite is composed once — on the snapshot it's routed and
/// prechecked on — and those calls are what run: a composer that would
/// say something else the second time is never asked, nested or not.
#[tokio::test(flavor = "multi_thread")]
async fn a_composite_runs_the_calls_it_was_routed_with() {
    use super::compose::{Compose, Composer, Composition};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (db, bus) = composing_bus();
    let composed = Arc::new(AtomicUsize::new(0));
    let count = composed.clone();
    let compose: Arc<Composer> = Arc::new(move |_conn, _trace, _input: &Value| {
        // The first composition sets `a`; any later one would set `b`.
        let k = if count.fetch_add(1, Ordering::SeqCst) == 0 {
            "a"
        } else {
            "b"
        };
        Ok(Composition {
            calls: vec![CommandCall {
                name: "oxplow.kv.set".into(),
                input: json!({ "k": k, "v": "1" }),
            }],
            ..Composition::default()
        })
    });
    let mut spec = kv_spec("oxplow.kv.once", Invokers::ALL, Confirm::Never);
    spec.atomicity = Atomicity::Dispatch;
    spec.input_schema = json!({ "type": "object" });
    bus.register(Command::new(spec, Compose::handler(compose)).unwrap())
        .unwrap();

    bus.run(&Actor::Human, "oxplow.kv.once", json!({}), false)
        .await
        .unwrap();
    assert_eq!(composed.load(Ordering::SeqCst), 1, "composed once");
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    assert_eq!(kv_value(&db, "b").await, None);

    // Nested in a sequence, in one transaction and as a step.
    for first in ["oxplow.kv.set", "oxplow.kv.external"] {
        composed.store(0, Ordering::SeqCst);
        bus.run(
            &Actor::Human,
            "oxplow.command.sequence",
            json!({ "calls": [
                { "name": first, "input": { "k": "x", "v": "1" } },
                { "name": "oxplow.kv.once", "input": {} },
            ] }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(composed.load(Ordering::SeqCst), 1, "{first}: composed once");
        assert_eq!(kv_value(&db, "b").await, None, "{first}");
    }
}

/// A step that is itself a composite whose child asks makes the run
/// ask before any step lands — not after the steps before it stand.
#[tokio::test(flavor = "multi_thread")]
async fn a_nested_composites_confirmation_is_asked_before_any_step_lands() {
    let (db, bus) = composing_bus();
    let input = json!({ "calls": [
        { "name": "oxplow.kv.external", "input": { "k": "a", "v": "1" } },
        { "name": "oxplow.command.sequence", "input": { "calls": [
            { "name": "oxplow.kv.danger", "input": { "k": "b", "v": "2" } },
        ] } },
    ] });
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            input.clone(),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
        "{err:?}"
    );
    assert_eq!(kv_value(&db, "a").await, None, "no step landed");
    bus.run(&Actor::Human, "oxplow.command.sequence", input, true)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
}

/// A child that changed nothing has nothing to undo: it doesn't keep
/// the composite from undoing what the others changed; a composite
/// whose every child changed nothing leaves no record, like such a
/// call.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_that_changed_nothing_doesnt_keep_the_composite_from_undoing() {
    let (db, bus) = composing_bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.same", Invokers::ALL, Confirm::Never),
            kv_unchanged(),
        )
        .unwrap(),
    )
    .unwrap();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.same", "b", "x")]),
            false,
        )
        .await
        .unwrap();
    assert!(out.inverse.is_some(), "undoable: the change to `a`");
    bus.undo(&Actor::Human, out.audit_id.unwrap(), false)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some(""));

    let before = audits_of(&db, "oxplow.command.sequence").await.len();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.same", "c", "x")]),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.audit_id, None, "nothing changed, nothing recorded");
    assert_eq!(
        audits_of(&db, "oxplow.command.sequence").await.len(),
        before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_run_applies_each_childs_own_checks() {
    let (db, bus) = composing_bus();
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(oxplow_domain::ThreadId::new(7)),
        stream_id: None,
    };
    let err = bus
        .run(
            &agent,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.secret", "b", "2")]),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(
        kv_value(&db, "a").await,
        None,
        "nothing ran: the pre-pass refused first"
    );

    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            json!({ "calls": [{ "name": "oxplow.kv.set", "input": { "k": "a", "v": "1" } }, { "name": "oxplow.kv.set", "input": { "k": "b" } }] }),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { field: Some(f), .. } if f.starts_with("/calls/1/input")),
        "{err:?}"
    );
    assert_eq!(kv_value(&db, "a").await, None);

    // A composite whose steps leave the transaction can't be one step
    // of another: its steps would land outside the outer run.
    let inner = calls(&[
        ("oxplow.kv.set", "a", "1"),
        ("oxplow.kv.external", "b", "2"),
    ]);
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            json!({ "calls": [{ "name": "oxplow.kv.set", "input": { "k": "c", "v": "3" } }, { "name": "oxplow.command.sequence", "input": inner }] }),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { message, .. } if message.contains("outside the transaction")),
        "{err:?}"
    );
    assert_eq!(kv_value(&db, "a").await, None);
    assert_eq!(kv_value(&db, "c").await, None);
}

/// The audit rows of `name`, newest first.
async fn audits_of(db: &Database, name: &str) -> Vec<oxplow_db::command_audit_store::CommandAudit> {
    oxplow_db::SqliteCommandAuditStore::new(db.clone())
        .list_recent(50)
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.command == name)
        .collect()
}

/// P7 review (tsk713): a composite with a step outside the transaction
/// runs its steps in order, each landing as it runs — one audit row
/// and one `command.executed` naming them all, and no undo (a system
/// outside the bus can't be rolled back with it).
#[tokio::test(flavor = "multi_thread")]
async fn a_composite_with_an_external_step_runs_its_steps_in_order() {
    let (db, bus) = composing_bus();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[
                ("oxplow.kv.set", "a", "1"),
                ("oxplow.kv.external", "b", "2"),
                ("oxplow.kv.set", "c", "3"),
            ]),
            false,
        )
        .await
        .unwrap();
    for (k, v) in [("a", "1"), ("b", "2"), ("c", "3")] {
        assert_eq!(kv_value(&db, k).await.as_deref(), Some(v), "{k}");
    }
    let names: Vec<&str> = out.result["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["oxplow.kv.set", "oxplow.kv.external", "oxplow.kv.set"]
    );
    assert!(out.inverse.is_none(), "not undoable");
    let rows = audits_of(&db, "oxplow.command.sequence").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome, Outcome::Ok);
    assert!(rows[0].inverse.is_none());
    for child in ["oxplow.kv.set", "oxplow.kv.external"] {
        assert!(
            audits_of(&db, child).await.is_empty(),
            "{child} has no row of its own"
        );
    }
}

/// P8.D4: a composite's own events, on the steps path, are recorded
/// with the run — caused by its `command.executed` — and only when
/// every step landed.
#[tokio::test(flavor = "multi_thread")]
async fn a_composites_own_events_land_with_its_steps_only_when_all_landed() {
    use super::compose::{Compose, Composer, Composition};
    let (db, bus) = composing_bus();
    let spec = CommandSpec {
        id: "oxplow.kv.announce".into(),
        summary: "Set over an external step and announce it.".into(),
        input_schema: json!({ "type": "object" }),
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: oxplow_domain::Lifecycle::Experimental,
        atomicity: oxplow_domain::Atomicity::Dispatch,
        access: oxplow_domain::Access::Write,
        needs: Vec::new(),
        ui: None,
        op: None,
        unrecorded: Vec::new(),
    };
    let compose: Arc<Composer> = Arc::new(|_conn, _trace, input: &Value| {
        Ok(Composition {
            calls: vec![CommandCall {
                name: "oxplow.kv.external".into(),
                input: input.clone(),
            }],
            result: None,
            events: vec![Envelope::typed::<
                oxplow_domain::events::schema::ConfigChanged,
            >(
                "test",
                &oxplow_domain::events::schema::ConfigChangedV2 {
                    key: "announced".into(),
                    before: Value::Null,
                    after: input["v"].clone(),
                    layer: oxplow_domain::events::schema::ConfigLayer::Project,
                },
            )],
        })
    });
    bus.register(Command::new(spec.clone(), Compose::handler(compose)).unwrap())
        .unwrap();
    let log = oxplow_db::SqliteEventLogStore::new(db.clone(), bus.vocabulary().clone());
    let announced = || async {
        log.read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == "config.changed")
            .collect::<Vec<_>>()
    };

    let out = bus
        .run(
            &Actor::Human,
            "oxplow.kv.announce",
            json!({ "k": "a", "v": "1" }),
            false,
        )
        .await
        .unwrap();
    let got = announced().await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].envelope.cause, out.event_id);

    bus.run(
        &Actor::Human,
        "oxplow.kv.announce",
        json!({ "k": "b", "v": "fail" }),
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(announced().await.len(), 1, "a failed run announces nothing");
}

/// P7 review (tsk713): a step that fails stops the run; the steps
/// before it stand, and the one audit row says what landed and what
/// failed.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_step_stops_the_run_and_what_landed_stands() {
    let (db, bus) = composing_bus();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[
                ("oxplow.kv.set", "a", "1"),
                ("oxplow.kv.external", "b", "fail"),
                ("oxplow.kv.set", "c", "3"),
            ]),
            false,
        )
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("`oxplow.kv.external`") && message.contains("`oxplow.kv.set`"),
        "names the failed step and what landed: {message}"
    );
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"), "it stands");
    assert_eq!(kv_value(&db, "c").await, None, "nothing after the failure");
    let rows = audits_of(&db, "oxplow.command.sequence").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome, Outcome::Error);
    let result = rows[0].result.clone().unwrap();
    assert_eq!(result["children"].as_array().unwrap().len(), 1);
    assert_eq!(result["failed"]["name"], "oxplow.kv.external");
    assert!(rows[0].inverse.is_none());
}

/// P7 review (tsk713): every step is checked before any runs — its
/// input, its invokers, the agent policy — and a step that asks makes
/// the run ask once; an agent's run is a proposal (with no dry run:
/// nothing outside the transaction runs before a person decides).
#[tokio::test(flavor = "multi_thread")]
async fn an_external_composite_is_checked_and_confirmed_before_any_step_runs() {
    let (db, bus) = composing_bus();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            json!({ "calls": [{ "name": "oxplow.kv.external", "input": { "k": "a", "v": "1" } }, { "name": "oxplow.kv.set", "input": { "k": "b" } }] }),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { field: Some(f), .. } if f.starts_with("/calls/1/input")),
        "{err:?}"
    );
    let err = bus
        .run(
            &agent(),
            "oxplow.command.sequence",
            calls(&[
                ("oxplow.kv.external", "a", "1"),
                ("oxplow.kv.secret", "b", "2"),
            ]),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    assert_eq!(kv_value(&db, "a").await, None, "nothing ran");

    let asks = calls(&[
        ("oxplow.kv.external", "a", "1"),
        ("oxplow.kv.danger", "b", "2"),
    ]);
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            asks.clone(),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
        "{err:?}"
    );
    let err = bus
        .run(&agent(), "oxplow.command.sequence", asks.clone(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    assert_eq!(
        kv_value(&db, "a").await,
        None,
        "nothing ran before a person decided"
    );

    bus.run(&Actor::Human, "oxplow.command.sequence", asks, true)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
}

/// A composite runs commands that write; a read (or a view) can't join it.
#[tokio::test(flavor = "multi_thread")]
async fn a_nested_run_refuses_a_read_child() {
    for access in [Access::View, Access::Read] {
        let (db, bus) = composing_bus();
        let mut read = kv_spec("oxplow.kv.peek", Invokers::ALL, Confirm::Never);
        read.access = access;
        bus.register(Command::new(read, kv_set()).unwrap()).unwrap();
        let err = bus
            .run(
                &Actor::Human,
                "oxplow.command.sequence",
                calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.peek", "b", "2")]),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/calls/1/name" && message.contains("only reads")),
            "{err}"
        );
        assert_eq!(kv_value(&db, "a").await, None);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_run_asks_when_any_child_asks() {
    let (db, bus) = composing_bus();
    let input = calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.danger", "b", "2")]);
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            input.clone(),
            false,
        )
        .await
        .unwrap_err();
    match err {
        CommandError::NeedsConfirmation { preview } => {
            assert!(preview.destructive);
            assert_eq!(preview.command, "oxplow.command.sequence");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        kv_value(&db, "a").await,
        None,
        "nothing written before the answer"
    );
    assert!(
        audits(&db).await.is_empty(),
        "no audit row for a confirmation"
    );
    bus.run(&Actor::Human, "oxplow.command.sequence", input, true)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_run_is_one_transaction_with_one_audit_row() {
    let (db, bus) = composing_bus();
    let err = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.set", "b", "half")]),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
    assert_eq!(
        kv_value(&db, "a").await,
        None,
        "the first child rolled back with the second"
    );

    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.set", "b", "2")]),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
    assert_eq!(out.result["children"][1]["result"]["v"], "2");
    let rows: Vec<_> = audits(&db)
        .await
        .into_iter()
        .filter(|a| a.outcome == Outcome::Ok)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "one audit row for the parent, none for the children"
    );
    assert_eq!(rows[0].command, "oxplow.command.sequence");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_runs_inverse_is_the_reversed_children_and_undoes() {
    let (db, bus) = composing_bus();
    bus.run(
        &Actor::Human,
        "oxplow.kv.set",
        json!({"k": "a", "v": "0"}),
        false,
    )
    .await
    .unwrap();
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.set", "b", "2")]),
            false,
        )
        .await
        .unwrap();
    let inverse = out.inverse.clone().expect("undoable");
    assert_eq!(inverse.name, "oxplow.command.sequence");
    assert_eq!(
        inverse.input["calls"],
        json!([
            { "name": "oxplow.kv.set", "input": { "k": "b", "v": "" } },
            { "name": "oxplow.kv.set", "input": { "k": "a", "v": "0" } }
        ])
    );
    bus.undo(&Actor::Human, out.audit_id.unwrap(), false)
        .await
        .unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("0"));
    assert_eq!(kv_value(&db, "b").await.as_deref(), Some(""));

    // A child that isn't undoable makes the parent not undoable.
    let out = bus
        .run(
            &Actor::Human,
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "c", "3"), ("oxplow.kv.plain", "d", "4")]),
            false,
        )
        .await
        .unwrap();
    assert!(out.inverse.is_none());
}

// P6b.A3: a run an agent needs a person to confirm is kept as a
// proposal; a person approves (it runs as them) or declines it.

async fn pending(db: &Database) -> Vec<oxplow_db::Proposal> {
    oxplow_db::SqliteProposalStore::new(db.clone())
        .list_pending()
        .await
        .unwrap()
}

async fn proposal(db: &Database, id: i64) -> oxplow_db::Proposal {
    oxplow_db::SqliteProposalStore::new(db.clone())
        .get(id)
        .await
        .unwrap()
        .unwrap()
}

async fn logged(bus: &CommandBus) -> Vec<oxplow_domain::events::StoredEvent> {
    bus.log_for_tests().read_after(0, 100).await.unwrap()
}

fn confirming_bus() -> (Database, CommandBus) {
    let (db, bus) = bus();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Always),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    (db, bus)
}

/// Propose `oxplow.kv.set k=v` as the agent; the proposal's id.
async fn propose(bus: &CommandBus, k: &str, v: &str) -> i64 {
    let err = bus
        .run(&agent(), "oxplow.kv.set", json!({ "k": k, "v": v }), false)
        .await
        .unwrap_err();
    let CommandError::Proposed { proposal, .. } = err else {
        panic!("{err:?}");
    };
    proposal
        .strip_prefix("proposal:")
        .and_then(|id| id.parse().ok())
        .unwrap()
}

/// A newer proposal of the same call replaces the pending one, and
/// says so: in its event and in the answer the agent gets.
#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_that_replaces_another_names_it() {
    let (_db, bus) = confirming_bus();
    let first = propose(&bus, "a", "1").await;
    let err = bus
        .run(
            &agent(),
            "oxplow.kv.set",
            json!({ "k": "a", "v": "1" }),
            false,
        )
        .await
        .unwrap_err();
    let CommandError::Proposed { supersedes, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(supersedes, &vec![format!("proposal:{first}")]);
    assert!(
        err.to_string()
            .contains(&format!("replaces proposal:{first}")),
        "{err}"
    );
    let proposed: Vec<_> = logged(&bus)
        .await
        .into_iter()
        .filter(|e| e.envelope.event_type == "command.proposed")
        .collect();
    assert_eq!(proposed[0].envelope.payload.get("supersedes"), None);
    assert_eq!(
        proposed[1].envelope.payload["supersedes"],
        json!([format!("proposal:{first}")])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_run_that_needs_confirmation_becomes_a_proposal() {
    let (db, bus) = confirming_bus();
    let err = bus
        .run(&agent(), "oxplow.kv.set", json!({"k": "a", "v": "1"}), true)
        .await
        .unwrap_err();
    let CommandError::Proposed {
        proposal, preview, ..
    } = err
    else {
        panic!("{err:?}");
    };
    assert_eq!(preview.command, "oxplow.kv.set");
    let rows = pending(&db).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(proposal, format!("proposal:{}", row.id));
    assert_eq!(row.command, "oxplow.kv.set");
    assert_eq!(row.actor_kind, ActorKind::Agent);
    assert_eq!(row.thread_id, Some(ThreadId::new(7)));
    assert_eq!(
        row.dry_run,
        Some(json!({"k": "a", "v": "1"})),
        "what it would have done"
    );
    assert_eq!(kv_value(&db, "a").await, None, "the dry run rolled back");
    assert!(audits(&db).await.is_empty(), "a proposal is not a run");
    let events = logged(&bus).await;
    assert_eq!(
        events
            .iter()
            .map(|e| e.envelope.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["command.proposed"],
        "only the proposal is logged"
    );
    assert_eq!(events[0].envelope.payload["proposal"], proposal);
    assert_eq!(events[0].envelope.payload["destructive"], false);
    assert!(events[0].envelope.subject.contains(&proposal));
    // A dry run that fails is the run's failure, not a proposal.
    let err = bus
        .run(
            &agent(),
            "oxplow.kv.set",
            json!({"k": "a", "v": "boom"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
    assert_eq!(pending(&db).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_composite_an_agent_runs_is_proposed_with_its_children() {
    let (db, bus) = composing_bus();
    let err = bus
        .run(
            &agent(),
            "oxplow.command.sequence",
            calls(&[("oxplow.kv.set", "a", "1"), ("oxplow.kv.danger", "b", "2")]),
            false,
        )
        .await
        .unwrap_err();
    let CommandError::Proposed { preview, .. } = err else {
        panic!("{err:?}");
    };
    assert!(preview.destructive);
    let rows = pending(&db).await;
    let children = rows[0].dry_run.as_ref().unwrap()["children"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(children.len(), 2);
    assert_eq!(children[1]["name"], "oxplow.kv.danger");
    assert_eq!(kv_value(&db, "a").await, None);
    assert!(audits(&db).await.is_empty());
    // Approving runs the whole composite as the person, once.
    let out = bus.approve(&Actor::Human, rows[0].id).await.unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    assert_eq!(kv_value(&db, "b").await.as_deref(), Some("2"));
    let p = proposal(&db, rows[0].id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
    assert_eq!(p.audit_id, out.audit_id);
    assert_eq!(audits(&db).await.len(), 1, "one row for the composite");
}

/// Two people approve one proposal at once: it runs once; the other
/// approval lost the race — `Invalid`, and no audit row of its own.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_approvals_run_it_once() {
    let (db, bus) = confirming_bus();
    let bus = Arc::new(bus);
    let id = propose(&bus, "a", "1").await;
    let (b1, b2) = (bus.clone(), bus.clone());
    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { b1.approve(&Actor::Human, id).await }),
        tokio::spawn(async move { b2.approve(&Actor::Human, id).await }),
    );
    let results = [r1.unwrap(), r2.unwrap()];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    let lost = results.into_iter().find_map(Result::err).unwrap();
    assert!(
        matches!(&lost, CommandError::Invalid { message, .. } if message.contains(&format!("proposal:{id}"))),
        "{lost:?}"
    );
    assert_eq!(audits(&db).await.len(), 1, "only the run that won");
    let approved = logged(&bus)
        .await
        .into_iter()
        .filter(|e| e.envelope.event_type == "command.approved")
        .count();
    assert_eq!(approved, 1);
}

/// An `External` approval whose run fails releases its claim: the
/// proposal is pending again and can be approved later.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_external_approval_leaves_the_proposal_pending() {
    let (db, bus) = bus();
    let mut spec = kv_spec("oxplow.kv.remote", Invokers::ALL, Confirm::Always);
    spec.atomicity = Atomicity::External;
    bus.register(
        Command::new(
            spec,
            Handler::External(Arc::new(|_actor, _input| {
                Box::pin(async move {
                    Err(CommandError::Failed {
                        message: "the remote said no".into(),
                    })
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    bus.run(
        &agent(),
        "oxplow.kv.remote",
        json!({"k": "a", "v": "1"}),
        false,
    )
    .await
    .unwrap_err();
    let id = pending(&db).await.remove(0).id;
    let err = bus.approve(&Actor::Human, id).await.unwrap_err();
    assert!(err.to_string().contains("the remote said no"), "{err}");
    let p = proposal(&db, id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Pending);
    assert_eq!(p.audit_id, None);
}

/// A dry run is rolled back with everything it would have caused: its
/// `after_commit` never runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_never_runs_after_commit() {
    let (db, bus) = bus();
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fired_c = fired.clone();
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.set", Invokers::ALL, Confirm::Always),
            Handler::Tx(Arc::new(move |_ctx: &TxCtx<'_>, input| {
                let fired = fired_c.clone();
                Ok(HandlerOutput {
                    result: input,
                    after_commit: Some(Box::new(move || {
                        fired.store(true, std::sync::atomic::Ordering::SeqCst)
                    })),
                    ..HandlerOutput::default()
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    propose(&bus, "a", "1").await;
    assert_eq!(pending(&db).await.len(), 1);
    assert!(!fired.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_runs_it_as_the_person_and_marks_the_proposal_with_the_run() {
    let (db, bus) = confirming_bus();
    let id = propose(&bus, "a", "1").await;
    let out = bus.approve(&Actor::Human, id).await.unwrap();
    assert_eq!(kv_value(&db, "a").await.as_deref(), Some("1"));
    let audit = out.audit_id.unwrap();
    let rows = audits(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, audit);
    assert_eq!(rows[0].actor_kind, ActorKind::Human, "it ran as the person");
    let p = proposal(&db, id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
    assert_eq!(p.audit_id, Some(audit));
    let approved = logged(&bus)
        .await
        .into_iter()
        .find(|e| e.envelope.event_type == "command.approved")
        .expect("approval logged");
    assert_eq!(approved.envelope.payload["audit_id"], audit);
    assert_eq!(approved.envelope.cause, out.event_id, "caused by the run");
    // A proposal is decided once.
    let err = bus.approve(&Actor::Human, id).await.unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { message, .. } if message.contains("already approved")),
        "{err:?}"
    );
    assert_eq!(audits(&db).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_whose_run_fails_leaves_the_proposal_pending() {
    let (db, bus) = confirming_bus();
    let id = propose(&bus, "a", "1").await;
    // The world moved: the key now exists and the table refuses it.
    db.transaction(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER kv_locked BEFORE INSERT ON kv BEGIN SELECT RAISE(ABORT, 'locked'); END;",
        )
        .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
    })
    .await
    .unwrap();
    bus.approve(&Actor::Human, id).await.unwrap_err();
    let p = proposal(&db, id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Pending);
    assert_eq!(p.audit_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_external_command_is_proposed_without_a_dry_run_and_approved_after_it_runs() {
    let (db, bus) = bus();
    let mut spec = kv_spec("oxplow.kv.remote", Invokers::ALL, Confirm::Always);
    spec.atomicity = Atomicity::External;
    bus.register(
        Command::new(
            spec,
            Handler::External(Arc::new(|_actor, input| {
                Box::pin(async move {
                    Ok(HandlerOutput {
                        result: input,
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &agent(),
            "oxplow.kv.remote",
            json!({"k": "a", "v": "1"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    let row = pending(&db).await.remove(0);
    assert_eq!(row.dry_run, None, "an External handler is never dry-run");
    let out = bus.approve(&Actor::Human, row.id).await.unwrap();
    let p = proposal(&db, row.id).await;
    assert_eq!(p.decision, oxplow_db::ProposalDecision::Approved);
    assert_eq!(p.audit_id, out.audit_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn declining_writes_only_the_decision() {
    let (db, bus) = confirming_bus();
    let id = propose(&bus, "a", "1").await;
    bus.decline(&Actor::Human, id).await.unwrap();
    assert_eq!(kv_value(&db, "a").await, None);
    assert!(audits(&db).await.is_empty());
    assert_eq!(
        proposal(&db, id).await.decision,
        oxplow_db::ProposalDecision::Declined
    );
    let declined = logged(&bus)
        .await
        .into_iter()
        .find(|e| e.envelope.event_type == "command.declined")
        .expect("decline logged");
    assert_eq!(
        declined.envelope.payload["proposal"],
        format!("proposal:{id}")
    );
    let err = bus.approve(&Actor::Human, id).await.unwrap_err();
    assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
}

fn effect() -> Actor {
    Actor::Effect {
        effect: "acme/notify".into(),
        thread_id: None,
        stream_id: None,
    }
}

/// P8.D8: an effect runs with an agent's rights — a command that asks
/// becomes a proposal (even when it says it confirmed), and a
/// person-only command is denied; the audit says it was the effect.
#[tokio::test(flavor = "multi_thread")]
async fn an_effect_runs_with_an_agents_rights_and_never_confirms() {
    let (db, bus) = confirming_bus();
    for confirmed in [false, true] {
        let err = bus
            .run(
                &effect(),
                "oxplow.kv.set",
                json!({ "k": "a", "v": "1" }),
                confirmed,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    }
    assert_eq!(kv_value(&db, "a").await, None);
    bus.register(
        Command::new(
            kv_spec("oxplow.kv.mine", Invokers::HUMAN_ONLY, Confirm::Never),
            kv_set(),
        )
        .unwrap(),
    )
    .unwrap();
    let err = bus
        .run(
            &effect(),
            "oxplow.kv.mine",
            json!({ "k": "b", "v": "1" }),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    let rows = audits_of(&db, "oxplow.kv.mine").await;
    assert_eq!(
        rows[0].actor_kind,
        oxplow_domain::events::schema::ActorKind::Effect
    );
    assert_eq!(rows[0].actor_id.as_deref(), Some("acme/notify"));
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_person_decides_a_proposal() {
    let (db, bus) = confirming_bus();
    let id = propose(&bus, "a", "1").await;
    let lens_for_agent = Actor::Lens {
        lens_id: "acme/x".into(),
        on_behalf_of: Box::new(agent()),
    };
    for actor in [agent(), lens_for_agent, Actor::System, effect()] {
        let err = bus.approve(&actor, id).await.unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = bus.decline(&actor, id).await.unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }
    assert_eq!(
        proposal(&db, id).await.decision,
        oxplow_db::ProposalDecision::Pending
    );
    assert_eq!(kv_value(&db, "a").await, None);
}
