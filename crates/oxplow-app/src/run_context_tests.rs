//! Every run of extension code is recorded once, as itself
//! (`.context/extensions.md` "The run context", `v_extension_run`): one
//! event reaches an `on:` collector and an effect; a person runs the
//! collector and the extension's command. Each run names its contribution,
//! the actor the host chose, what caused it and where, and the records
//! that already existed (`collector_run`, `effect_run`, `command_audit`)
//! point at it.

use std::path::Path;
use std::sync::Arc;

use oxplow_domain::events::{Anchors, Envelope, StoredEvent};
use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::collector_runner::{run_for_event, Collectors};
use crate::exec_consent::ProgramKind;
use crate::Services;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

/// `acme`: an event type, an `on:` collector, an effect and a command.
fn acme(root: &Path, task: &str) {
    write(
        root,
        "oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nversion: 1.2.0\nsharing: private\nintent: { purpose: Runs., origin: null, examples: [] }\n\
         event_types:\n  types:\n    - { type: acme.merged, v: 1, schema: merged.json, summary: A merge. }\n\
         collectors:\n\
         \x20 - id: star\n    runtime: starlark\n    entry: star.star\n    trigger: { on: [acme.merged] }\n    entities:\n      - { name: seen, key: k, columns: { k: text } }\n\
         effects:\n\
         \x20 - id: note\n    summary: Note a merge.\n    on: [acme.merged]\n    entry: note.star\n\
         commands:\n\
         \x20 - name: note.write\n    summary: Note it.\n    input_schema: { type: object }\n    entry: write.star\n    invokers: { human: true, agent: true, lens: true }\n",
    );
    write(
        root,
        "oxplow/extensions/acme/merged.json",
        r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
    );
    write(
        root,
        "oxplow/extensions/acme/star.star",
        "def transform(input):\n    return {\"entities\": {\"seen\": [{\"k\": \"x\"}]}}\n",
    );
    let compose = format!(
        "{{\"commands\": [{{\"name\": \"oxplow.work_item.update\", \"input\": {{\"ref\": \"{task}\", \"title\": \"merged\"}}}}]}}"
    );
    write(
        root,
        "oxplow/extensions/acme/note.star",
        &format!("def transform(x):\n    return {compose}\n"),
    );
    write(
        root,
        "oxplow/extensions/acme/write.star",
        &format!("def transform(x):\n    return {compose}\n"),
    );
}

async fn rows(svc: &Services, sql: &str) -> Vec<Vec<Value>> {
    let sql = sql.to_string();
    svc.db
        .read(move |c| {
            let mut s = c.prepare(&sql).map_err(oxplow_db::map_sql_err)?;
            let n = s.column_count();
            let rows = s
                .query_map([], |r| {
                    (0..n)
                        .map(|i| {
                            r.get::<_, rusqlite::types::Value>(i).map(|v| match v {
                                rusqlite::types::Value::Null => Value::Null,
                                rusqlite::types::Value::Integer(i) => json!(i),
                                rusqlite::types::Value::Real(f) => json!(f),
                                rusqlite::types::Value::Text(t) => json!(t),
                                rusqlite::types::Value::Blob(_) => json!("blob"),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .map_err(oxplow_db::map_sql_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows)
        })
        .await
        .unwrap()
}

/// Each run is one `extension_run` row: its contribution at its version
/// and generation, the actor its host chose, its cause and anchors; the
/// rows that already recorded it name it.
#[tokio::test(flavor = "multi_thread")]
async fn every_run_is_recorded_once_as_itself() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    let task = oxplow_tasks::work_item_ref(fx.task);
    acme(&root, &task);
    svc.vocabulary_service.sync().await.unwrap();
    svc.extension_commands.reconcile().await;
    let generation = svc.extension_catalog.generation(&root);

    let env = Envelope::new("acme.merged", 1, "test", json!({ "number": 1 }))
        .unwrap()
        .with_anchors(Anchors {
            thread_id: Some(fx.thread),
            ..Anchors::default()
        })
        .with_subject([task.clone()]);
    let id = env.id.clone();
    svc.event_log_store.append(env).await.unwrap();
    let event: StoredEvent = svc.event_log_store.get(id).await.unwrap().unwrap();
    let event_id = event.envelope.id.to_string();

    // The `on:` collector, as the system, caused by the event.
    run_for_event(
        &Collectors::of(svc, &root),
        "acme",
        "star",
        Arc::new(event.clone()),
    )
    .await
    .unwrap();
    let collector = rows(
        svc,
        "SELECT r.version, r.generation, r.actor, r.cause_event_id, r.thread_id, r.outcome,
                r.kind, (SELECT run_id FROM collector_run WHERE owner = 'acme' AND id = 'star') = r.run_id
           FROM extension_run r WHERE r.contribution = 'acme/collector/star'",
    )
    .await;
    assert_eq!(
        collector,
        vec![vec![
            json!("1.2.0"),
            json!(generation),
            json!("system"),
            json!(event_id),
            json!(fx.thread.value()),
            json!("ok"),
            json!("collector"),
            json!(1),
        ]]
    );

    // The effect, as itself, caused by the event; its attempt and the run
    // of what it composed name its run.
    let config = svc.config.read().unwrap().clone();
    let (ext, decl) = crate::effect_triggers::effects(svc)
        .into_iter()
        .find(|(e, _)| e.name == "acme")
        .unwrap();
    let program = crate::effects::effect_program(&ext, &decl);
    crate::exec_consent::approve_program(
        &svc.approvals,
        &root,
        &config,
        std::slice::from_ref(&ext),
        ProgramKind::Effect,
        &program.name,
        &program.hash(&root).unwrap(),
    )
    .unwrap();
    crate::effects::approved(&svc.db, &decl.name())
        .await
        .unwrap();
    // Logged after the approval: an effect never reacts to the past.
    let env = Envelope::new("acme.merged", 1, "test", json!({ "number": 2 }))
        .unwrap()
        .with_anchors(Anchors {
            thread_id: Some(fx.thread),
            ..Anchors::default()
        })
        .with_subject([task.clone()]);
    let id = env.id.clone();
    svc.event_log_store.append(env).await.unwrap();
    let second = svc.event_log_store.get(id).await.unwrap().unwrap();
    let consumer = Arc::new(crate::effect_triggers::EffectTriggers::new(Arc::downgrade(
        svc,
    )));
    crate::event_pump::run_async_handler(
        svc.vocabulary.clone(),
        consumer,
        Arc::new(second.clone()),
    )
    .await
    .unwrap();
    let effect = rows(
        svc,
        "SELECT r.actor, r.cause_event_id, r.outcome,
                (SELECT count(*) FROM effect_run e WHERE e.run_id = r.run_id),
                (SELECT count(*) FROM command_audit a WHERE a.run_id = r.run_id)
           FROM extension_run r WHERE r.contribution = 'acme/effect/note'",
    )
    .await;
    assert_eq!(
        effect,
        vec![vec![
            json!("effect:acme/note"),
            json!(second.envelope.id.to_string()),
            json!("ok"),
            json!(1),
            json!(1),
        ]]
    );

    // A person runs the collector: its run is nested in theirs.
    svc.commands
        .run(
            &Actor::Human,
            crate::collector_runner::SYNC,
            json!({ "owner": "acme", "id": "star" }),
            false,
        )
        .await
        .unwrap();
    let nested = rows(
        svc,
        "SELECT r.actor, a.command FROM extension_run r
           JOIN command_audit a ON a.run_id = r.parent_run_id
          WHERE r.contribution = 'acme/collector/star' AND r.parent_run_id IS NOT NULL",
    )
    .await;
    assert_eq!(
        nested,
        vec![vec![json!("human"), json!(crate::collector_runner::SYNC)]]
    );

    // A person runs the extension's command: the command's run, audited
    // under it.
    svc.commands
        .run(&Actor::Human, "acme.note.write", json!({}), false)
        .await
        .unwrap();
    let command = rows(
        svc,
        "SELECT r.actor, r.kind, r.outcome, a.command FROM extension_run r
           JOIN command_audit a ON a.run_id = r.run_id
          WHERE r.contribution = 'acme/command/note.write'",
    )
    .await;
    assert_eq!(
        command,
        vec![vec![
            json!("human"),
            json!("command"),
            json!("ok"),
            json!("acme.note.write")
        ]]
    );
}
