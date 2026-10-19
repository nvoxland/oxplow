//! The services a script uses, through every kind of host
//! (`.context/extensions.md` "Services"): a collector reads the tree it
//! declares, a fact collector reads the semantic layer, a command's script
//! can't ask a model — and what was decided lands on its run.

use std::path::Path;
use std::sync::{Arc, Mutex};

use oxplow_domain::events::Anchors;
use oxplow_domain::run::{Mode, RunContext};
use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::collector_runner::{run_collector, Collectors, RunCollectorError, RunTrigger};
use crate::services::{Tree, TreeSource};
use crate::Services;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

/// A tree source that answers one tree and remembers what it was asked.
#[derive(Default)]
struct Trees {
    asked: Mutex<Vec<Anchors>>,
}

#[async_trait::async_trait]
impl TreeSource for Trees {
    async fn tree(&self, anchors: &Anchors) -> Result<Tree, String> {
        self.asked.lock().unwrap().push(anchors.clone());
        Ok(Arc::new(
            [
                ("src/a.rs".to_string(), "fn a() {}\n".to_string()),
                ("docs/b.md".to_string(), "# b\n".to_string()),
            ]
            .into(),
        ))
    }
}

const PATHS: &str =
    "def transform(input):\n    return {\"entities\": {\"seen\": [{\"k\": f[\"path\"]} for f in files(\"src/**\")]}}\n";

/// What the run of `contribution` decided, as recorded.
async fn decided(svc: &Services, contribution: &str) -> Value {
    let contribution = contribution.to_string();
    let text: String = svc
        .db
        .read(move |c| {
            c.query_row(
                "SELECT decisions_json FROM extension_run WHERE contribution = ?1",
                [&contribution],
                |r| r.get(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// An entity collector that declares `tree.read` reads the tree its run
/// is anchored to; one that uses it without declaring it doesn't load.
#[tokio::test(flavor = "multi_thread")]
async fn a_collector_reads_the_tree_it_declares() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent: { purpose: Reads., origin: null, examples: [] }\n\
         permissions: { files: read }\n\
         collectors:\n\
         \x20 - id: paths\n    runtime: starlark\n    entry: paths.star\n    needs: [tree.read]\n    entities:\n      - { name: seen, key: k, columns: { k: text } }\n",
    );
    write(&root, ".oxplow/extensions/acme/paths.star", PATHS);
    crate::test_fixtures::activate(svc).await;
    let trees = Arc::new(Trees::default());
    let mut collectors = Collectors::of(svc, &root);
    collectors.trees = Some(trees.clone());
    let as_person = RunContext::core(Actor::Human, Mode::External);

    let report = run_collector(&collectors, &as_person, "acme", "paths", RunTrigger::Manual)
        .await
        .unwrap();
    assert_eq!(report.row_counts["seen"], 1);
    assert_eq!(trees.asked.lock().unwrap().len(), 1);
    assert_eq!(
        decided(svc, "collector:acme/paths").await,
        json!([{ "need": "scope", "name": "tree.read", "decision": "allowed" }])
    );

    // One that uses the tree without declaring it doesn't load: the
    // edit is held back and the working version stays.
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent: { purpose: Reads., origin: null, examples: [] }\n\
         permissions: { files: read }\n\
         collectors:\n\
         \x20 - id: paths\n    runtime: starlark\n    entry: paths.star\n    needs: [tree.read]\n    entities:\n      - { name: seen, key: k, columns: { k: text } }\n\
         \x20 - id: peek\n    runtime: starlark\n    entry: paths.star\n    entities:\n      - { name: peeked, key: k, columns: { k: text } }\n",
    );
    let errors = crate::test_fixtures::load_errors(svc, "acme").await;
    assert!(
        errors.contains("uses `files()` without `tree.read` in its `needs`"),
        "{errors}"
    );
    assert!(matches!(
        run_collector(&collectors, &as_person, "acme", "peek", RunTrigger::Manual).await,
        Err(RunCollectorError::NotFound)
    ));
}

/// A command's script composes, so it can't ask a model, and the bus
/// composes it inside its transaction, so it can't read the tree: each is
/// refused at load, naming why. (Its run's refusal of a model, had one
/// got through: `services::tests::a_pure_run_cant_ask_a_model`.)
#[tokio::test(flavor = "multi_thread")]
async fn a_commands_script_cant_ask_a_model_or_read_the_tree() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    let errors_with = |needs: &str, body: &str| {
        write(
            &root,
            ".oxplow/extensions/acme/extension.yaml",
            &format!(
                "manifest: 2\nname: acme\nintent: {{ purpose: Asks., origin: null, examples: [] }}\n\
                 commands:\n\
                 \x20 - name: note.sum\n    summary: Sum.\n    input_schema: {{ type: object }}\n    entry: sum.star\n    needs: [{needs}]\n    invokers: {{ human: true, agent: true, lens: true }}\n"
            ),
        );
        write(
            &root,
            ".oxplow/extensions/acme/sum.star",
            &format!("def transform(x):\n    return {{\"commands\": [], \"result\": {body}}}\n"),
        );
        crate::test_fixtures::load_errors(svc, "acme")
    };
    let errors = errors_with("sql.read", "ai_summarize(\"t\")").await;
    assert!(
        errors.contains(
            "uses `ai_summarize()`: `needs`: `ai.compute` (ask a model) can't be used here"
        ),
        "{errors}"
    );
    let errors = errors_with("tree.read", "len(files(\"**\"))").await;
    assert!(
        errors.contains("a command's script decides what runs"),
        "{errors}"
    );
    let held = crate::test_fixtures::activate_with_problems(svc).await;
    assert!(held.contains_key("acme"), "{held:?}");
    assert!(svc.commands.spec("acme.note.sum").is_none());
}

/// A fact collector that declares `sql.read` reads the semantic layer
/// inside its script; one that asks a model runs only once approved.
#[tokio::test(flavor = "multi_thread")]
async fn a_fact_collector_reads_the_layer_it_declares() {
    let fx = crate::test_fixtures::services_with_task_effort().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent: { purpose: Counts., origin: null, examples: [] }\n\
         permissions: { sql: read, ai: true }\n\
         collectors:\n\
         \x20 - id: items\n    runtime: starlark\n    entry: items.star\n    needs: [sql.read]\n    facts: [acme.items]\n\
         \x20 - id: asks\n    runtime: starlark\n    entry: asks.star\n    needs: [ai.compute]\n    facts: [acme.items]\n",
    );
    write(
        &root,
        ".oxplow/extensions/acme/items.star",
        "def transform(x):\n    rows = scope(\"sql.read\", {\"sql\": \"SELECT count(*) AS n FROM v_work_item\"})\n    return {\"facts\": [{\"measure\": \"acme.items\", \"value\": rows[0][\"n\"] + len(files(\"**\"))}]}\n",
    );
    write(
        &root,
        ".oxplow/extensions/acme/asks.star",
        "def transform(x):\n    return {\"facts\": [{\"measure\": \"acme.items\", \"value\": len(ai_summarize(\"t\"))}]}\n",
    );
    crate::test_fixtures::activate(svc).await;
    let ext = svc
        .extension_catalog
        .unpinned(&root, "a test reads declarations")
        .iter()
        .find(|e| e.name == "acme")
        .cloned()
        .unwrap();
    let spec = |id: &str| ext.collectors.iter().find(|c| c.id == id).unwrap().clone();
    let files = [("a.rs".to_string(), "fn a() {}".to_string())].into();
    let facts = svc
        .metrics
        .preview_fact_collector("acme", &spec("items"), files, json!({}), Mode::Pure)
        .await
        .unwrap();
    // The task fixture's one item, and the one file it was given.
    assert_eq!(facts[0]["value"], 2, "{facts:?}");

    let err = svc
        .metrics
        .preview_fact_collector(
            "acme",
            &spec("asks"),
            Default::default(),
            json!({}),
            Mode::Pure,
        )
        .await
        .unwrap_err();
    assert!(err.contains("needs a person's approval first"), "{err}");
}
