//! tsk1047: `oxplow plugin test` runs a fact collector's examples offline,
//! over the files an example gives it, and compares the facts — the walk's
//! todo-watch collector could only be tested live.

#![allow(clippy::unwrap_used)]

use oxplow_sdk::plugin_test::test_extension;

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fact_collectors_examples_run_over_their_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let ext = "oxplow/extensions/todos";
    write(
        root,
        &format!("{ext}/extension.yaml"),
        "manifest: 2\n\
         name: todos\n\
         sharing: private\n\
         intent:\n  purpose: Count TODO comments.\n  origin: null\n  examples:\n    - { name: one-todo }\n    - { name: wrong-count }\n\
         measures:\n  - { key: todos.count, title: TODO comments, subjectKind: file, captureScope: per-path }\n\
         collectors:\n  - id: todos.scan\n    doc: TODO comments per file\n    runtime: starlark\n    entry: collectors/scan.star\n    trigger: { on: [snapshot.taken] }\n    facts: [todos.count]\n",
    );
    write(
        root,
        &format!("{ext}/collectors/scan.star"),
        "def transform(input):\n    facts = []\n    for f in files(\"**/*.ts\"):\n        n = f[\"text\"].count(\"TODO\")\n        if n:\n            facts.append({\"measure\": \"todos.count\", \"value\": n, \"subject\": \"file:\" + f[\"path\"], \"path\": f[\"path\"]})\n    return {\"facts\": facts}\n",
    );
    write(
        root,
        &format!("{ext}/fixtures/one-todo.yaml"),
        "input:\n  collector: todos.scan\n  files:\n    src/a.ts: \"// TODO: one\\nexport const a = 1;\\n\"\n    src/b.ts: \"export const b = 2;\\n\"\n\
         expect:\n  facts:\n    - { measure: todos.count, value: 1, path: src/a.ts }\n",
    );
    write(
        root,
        &format!("{ext}/fixtures/wrong-count.yaml"),
        "input:\n  collector: todos.scan\n  files:\n    src/a.ts: \"// TODO: one\\n\"\n\
         expect:\n  facts:\n    - { measure: todos.count, value: 2, path: src/a.ts }\n",
    );
    let tested = test_extension(root, "todos", false).await.unwrap();
    assert!(
        tested.ran.contains(&"example one-todo".to_string()),
        "{:?}",
        tested.ran
    );
    assert!(
        tested.ran.contains(&"example wrong-count".to_string()),
        "{:?}",
        tested.ran
    );
    assert_eq!(tested.errors.len(), 1, "{:?}", tested.errors);
    assert!(
        tested.errors[0].contains("wrong-count"),
        "{:?}",
        tested.errors
    );
    assert!(tested.errors[0].contains("facts"), "{:?}", tested.errors);
}
