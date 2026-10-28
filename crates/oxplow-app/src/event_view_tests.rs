//! The event envelope is the same on every path that hands an event to an
//! extension (`.context/extensions.md` "The event envelope"): one event,
//! logged at v1 of a type whose owner has since published v2, with anchors,
//! a subject and a cause, read by a derived collector, an exec collector
//! (on its stdin) and an effect — each sees exactly `EventView::of` it, at
//! v2, and a fixture of the same facts builds the same thing.

use std::path::Path;
use std::sync::Arc;

use oxplow_domain::events::{Anchors, Envelope, EventView, StoredEvent};
use serde_json::{json, Value};

use crate::collector_runner::{run_for_event, Collectors};
use crate::Services;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    #[cfg(unix)]
    if rel.ends_with(".sh") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

const OWNER_HEAD: &str = "manifest: 2\nname: acme-pr\nintent: { purpose: PRs., origin: null, examples: [] }\nevent_types:\n  types:\n";
const V1: &str = "    - { type: acme_pr.merged, v: 1, schema: merged.v1.json, summary: A pull request merged. }\n";
const V2: &str = "    - { type: acme_pr.merged, v: 2, schema: merged.v2.json, summary: A pull request merged by someone., upcast: merged.star }\n";

/// `acme-pr` declaring `acme_pr.merged` (v1 only, or v1 and v2), and
/// `acme`, which reads it three ways.
fn extensions(root: &Path, with_v2: bool) {
    write(
        root,
        ".oxplow/extensions/acme-pr/extension.yaml",
        &format!("{OWNER_HEAD}{V1}{}", if with_v2 { V2 } else { "" }),
    );
    write(
        root,
        ".oxplow/extensions/acme-pr/merged.v1.json",
        r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme-pr/merged.v2.json",
        r#"{"type": "object", "required": ["number", "by"], "properties": {"number": {"type": "integer"}, "by": {"type": "string"}}}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme-pr/merged.star",
        "def transform(x):\n    p = dict(x[\"payload\"])\n    p[\"by\"] = \"unknown\"\n    return p\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent: { purpose: Reads merges., origin: null, examples: [] }\n\
         permissions: [process.run]\n\
         collectors:\n\
         \x20 - id: star\n    runtime: starlark\n    entry: star.star\n    on: [acme_pr.merged]\n    entities:\n      - { name: seen_star, key: k, columns: { k: text, event: text } }\n\
         \x20 - id: prog\n    runtime: exec\n    entry: prog.sh\n    on: [acme_pr.merged]\n    entities:\n      - { name: seen_prog, key: k, columns: { k: text, event: text } }\n\
         effects:\n\
         \x20 - id: note\n    summary: Note a merge.\n    on: [acme_pr.merged]\n    entry: note.star\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/star.star",
        "def transform(input):\n    return {\"entities\": {\"seen_star\": [{\"k\": \"x\", \"event\": json.encode(input[\"event\"])}]}}\n",
    );
    // The program's stdin, as one escaped JSON string.
    write(
        root,
        ".oxplow/extensions/acme/prog.sh",
        "#!/bin/sh\nbody=$(sed 's/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g')\nprintf '{\"entities\":{\"seen_prog\":[{\"k\":\"x\",\"event\":\"%s\"}]}}' \"$body\"\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/note.star",
        "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.update\", \"input\": {\"uri\": \"work_item:oxplow:tsk1\", \"title\": json.encode(x[\"event\"])}}]}\n",
    );
}

async fn log(svc: &Services, env: Envelope) -> StoredEvent {
    let id = env.id.clone();
    svc.event_log_store.append(env).await.unwrap();
    svc.event_log_store.get(id).await.unwrap().unwrap()
}

async fn seen(svc: &Services, view: &str) -> Value {
    let rows = svc
        .sql
        .run(oxplow_db::SqlQuery::new(format!(
            "SELECT event FROM {view}"
        )))
        .await
        .unwrap();
    match &rows.rows[..] {
        [row] => match &row[0] {
            oxplow_db::SqlCell::Text(t) => serde_json::from_str(t).unwrap(),
            other => panic!("{other:?}"),
        },
        other => panic!("{view}: {other:?}"),
    }
}

/// The same logged event reaches a derived collector, an exec collector
/// and an effect as one envelope — every field, at the newest version.
#[tokio::test]
async fn every_path_hands_over_the_same_envelope() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    extensions(&root, false);
    crate::test_fixtures::activate_with_problems(svc).await;

    // Logged at v1 with every kind of provenance, while v1 was newest.
    let cause = log(
        svc,
        Envelope::new("acme_pr.merged", 1, "test", json!({ "number": 11 })).unwrap(),
    )
    .await;
    let event = log(
        svc,
        Envelope::new(
            "acme_pr.merged",
            1,
            "oxplow:oxplow/collector?collector=acme-pr/gh",
            json!({ "number": 12 }),
        )
        .unwrap()
        .with_anchors(Anchors {
            thread_id: Some(fx.thread),
            effort_id: Some(fx.effort),
            turn_id: Some(7),
            agent_session_id: Some(fx.session),
            ..Anchors::default()
        })
        .with_subject([oxplow_tasks::work_item_ref(fx.task)])
        .with_cause(cause.envelope.id.clone()),
    )
    .await;
    // The owner publishes v2; every reader gets the row carried to it.
    extensions(&root, true);
    crate::test_fixtures::activate_with_problems(svc).await;
    let expected = EventView::of(&event, &svc.vocabulary.current()).unwrap();
    assert_eq!(expected.v, 2);
    assert_eq!(expected.payload, json!({ "number": 12, "by": "unknown" }));
    let expected = expected.to_json();

    // A fixture of the same facts is the same envelope.
    let mut given = expected.clone();
    given["v"] = json!(1);
    given["payload"] = json!({ "number": 12 });
    assert_eq!(
        EventView::fixture(&given, &svc.vocabulary.current())
            .unwrap()
            .to_json(),
        expected
    );

    // A derived collector's `input.event`.
    let collectors = Collectors::of(svc, &root);
    run_for_event(&collectors, "acme", "star", Arc::new(event.clone()))
        .await
        .unwrap();
    assert_eq!(seen(svc, "v_acme_seen_star").await, expected);

    // An exec collector's stdin: `{ event }`.
    run_for_event(&collectors, "acme", "prog", Arc::new(event.clone()))
        .await
        .unwrap();
    assert_eq!(
        seen(svc, "v_acme_seen_prog").await,
        json!({ "event": expected })
    );

    // An effect's `x["event"]`.
    let (_, decl) = crate::effect_triggers::effects(svc)
        .into_iter()
        .find(|(e, d)| e.name == "acme" && d.id == "note")
        .unwrap();
    let (ext, _) = crate::effect_triggers::effects(svc)
        .into_iter()
        .find(|(e, d)| e.name == "acme" && d.id == "note")
        .unwrap();
    let run = crate::run_context::for_effect(
        &svc.extension_catalog,
        &svc.layout.project_dir,
        &ext,
        &decl,
        &event,
    );
    let (reaction, _) = crate::effect_triggers::run_script(svc, &run, &decl, &event)
        .await
        .unwrap();
    let crate::effects::Reaction::Run { calls, .. } = reaction else {
        panic!("{reaction:?}");
    };
    let title = calls[0].input["title"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(title).unwrap(), expected);
}
