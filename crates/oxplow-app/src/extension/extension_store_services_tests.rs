//! End to end: an extension's own SQLite file and declared folders. Scripts
//! read with `scope("store.sql", …)` and `scope("files.read", …)`; they write
//! by composing `oxplow.extension_store.write` and
//! `oxplow.extension_files.write`, which run through the real command bus.

use std::path::Path;

use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use super::extension_private_state_tests::{reconcile, run, write};
use crate::test_fixtures::{services_with_thread, ThreadFixture};

fn declare(root: &Path, name: &str) {
    let manifest = format!(
        "manifest: 2
name: {name}
intent:
  purpose: p
permissions: [files.write, 'commands:oxplow.extension_store.write', 'commands:oxplow.extension_files.*']
storage: {{ sqlite: true }}
files:
  - {{ path: shelf-{name}, access: write }}
  - {{ path: docs, access: read }}
commands:
  - name: book.put
    summary: Store a book and its file.
    input_schema: {{ type: object }}
    entry: handlers/put.star
    needs: [store.sql]
  - name: book.mint
    summary: Store a book under an id the store mints.
    input_schema: {{ type: object }}
    entry: handlers/mint.star
    needs: [store.sql]
  - name: book.put_doc
    summary: Write into the read-only folder.
    input_schema: {{ type: object }}
    entry: handlers/put_doc.star
  - name: book.peek
    summary: Read the store and a file.
    input_schema: {{ type: object }}
    entry: handlers/peek.star
    needs: [store.sql, files.read]
  - name: book.escape
    summary: Read outside the folders.
    input_schema: {{ type: object }}
    entry: handlers/escape.star
    needs: [files.read]
  - name: book.mutate
    summary: Try to write through the read service.
    input_schema: {{ type: object }}
    entry: handlers/mutate.star
    needs: [store.sql]
"
    );
    let base = format!(".oxplow/extensions/{name}");
    write(root, &format!("{base}/extension.yaml"), &manifest);
    write(
        root,
        &format!("{base}/storage/migrations/V1__book.sql"),
        "CREATE TABLE book (id TEXT PRIMARY KEY, title TEXT NOT NULL);",
    );
    write(
        root,
        &format!("{base}/storage/migrations/V2__seed.sql"),
        "INSERT INTO book (id, title) VALUES ('s0', 'Seed');",
    );
    let put = format!(
        "def transform(x):
    return {{\"commands\": [
        {{\"name\": \"oxplow.extension_store.write\", \"input\": {{\"batch\": [
            {{\"sql\": \"INSERT INTO book (id, title) VALUES (?1, ?2)\", \"params\": [x[\"input\"][\"id\"], x[\"input\"][\"title\"]]}},
        ]}}}},
        {{\"name\": \"oxplow.extension_files.write\", \"input\": {{\"path\": \"shelf-{name}/\" + x[\"input\"][\"id\"] + \".md\", \"body\": x[\"input\"][\"title\"]}}}},
    ], \"result\": scope(\"store.sql\", {{\"sql\": \"SELECT count(*) AS n FROM book\"}})}}
"
    );
    write(root, &format!("{base}/handlers/put.star"), &put);
    write(
        root,
        &format!("{base}/handlers/mint.star"),
        "def transform(x):
    return {\"commands\": [
        {\"name\": \"oxplow.extension_store.write\", \"input\": {\"batch\": [
            {\"sql\": \"INSERT INTO book (id, title) VALUES ('m-' || (SELECT count(*) FROM book), ?1) RETURNING id, title\", \"params\": [x[\"input\"][\"title\"]]},
            {\"sql\": \"UPDATE book SET title = title WHERE id = 's0'\"},
        ]}},
    ], \"result\": scope(\"store.sql\", {\"sql\": \"SELECT count(*) AS n FROM book\"})}
",
    );
    write(
        root,
        &format!("{base}/handlers/put_doc.star"),
        "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.extension_files.write\", \"input\": {\"path\": \"docs/a.md\", \"body\": \"x\"}}], \"result\": {}}\n",
    );
    let peek = format!(
        "def transform(x):
    return {{\"commands\": [], \"result\": {{
        \"books\": scope(\"store.sql\", {{\"sql\": \"SELECT id, title FROM book WHERE id = ?1\", \"params\": [x[\"input\"][\"id\"]]}}),
        \"file\": scope(\"files.read\", {{\"path\": \"shelf-{name}/\" + x[\"input\"][\"id\"] + \".md\"}}),
        \"listed\": scope(\"files.list\", {{\"path\": \"shelf-{name}\"}}),
    }}}}
"
    );
    write(root, &format!("{base}/handlers/peek.star"), &peek);
    write(
        root,
        &format!("{base}/handlers/escape.star"),
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"files.read\", {\"path\": x[\"input\"][\"path\"]})}\n",
    );
    write(
        root,
        &format!("{base}/handlers/mutate.star"),
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"store.sql\", {\"sql\": \"DELETE FROM book\"})}\n",
    );
}

async fn fixture() -> ThreadFixture {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha");
    declare(fx._dir.path(), "beta");
    reconcile(&fx).await;
    fx
}

fn store_file(fx: &ThreadFixture, name: &str) -> std::path::PathBuf {
    fx._dir
        .path()
        .join(format!(".oxplow/extensions-data/{name}.sqlite"))
}

/// A store write answers each statement's `RETURNING` rows, so a store
/// can mint an id in its write. They are in the run's result, on the
/// write's child; the script's own `result` is composed before any step
/// runs, so it can't see them (nothing binds an earlier step's answer into
/// a later one).
#[tokio::test(flavor = "multi_thread")]
async fn a_store_write_answers_its_returning_rows_in_the_runs_children() {
    let fx = fixture().await;
    let out = run(&fx, "alpha.book.mint", json!({"title": "Dune"}))
        .await
        .unwrap();
    assert_eq!(out["children"][0]["name"], "oxplow.extension_store.write");
    assert_eq!(
        out["children"][0]["result"],
        json!({ "changes": 2, "rows": [[{ "id": "m-1", "title": "Dune" }], []] }),
        "{out}"
    );
    assert_eq!(
        out["result"],
        json!({ "rows": [{ "n": 1 }] }),
        "the script's result was read before the write ran: {out}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_writes_the_store_and_a_file_by_composing_commands_and_reads_them_back() {
    let fx = fixture().await;
    let put = run(&fx, "alpha.book.put", json!({"id": "b1", "title": "Dune"}))
        .await
        .unwrap();
    // The seed migration ran once: the seed and the new book.
    assert_eq!(
        put["result"],
        json!({ "rows": [{ "n": 1 }] }),
        "read before the write lands: {put}"
    );
    let peek = run(&fx, "alpha.book.peek", json!({"id": "b1"}))
        .await
        .unwrap();
    assert_eq!(
        peek["result"]["books"]["rows"],
        json!([{ "id": "b1", "title": "Dune" }])
    );
    assert_eq!(peek["result"]["file"], json!({ "body": "Dune" }));
    assert_eq!(
        peek["result"]["listed"]["files"][0]["path"],
        "shelf-alpha/b1.md"
    );
    assert!(store_file(&fx, "alpha").exists());
    assert!(fx._dir.path().join("shelf-alpha/b1.md").exists());

    // Migrations run once: reactivating does not run the seed again.
    reconcile(&fx).await;
    let count = run(&fx, "alpha.book.put", json!({"id": "b2", "title": "Emma"}))
        .await
        .unwrap();
    assert_eq!(count["result"], json!({ "rows": [{ "n": 2 }] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_batch_leaves_the_store_as_it_was() {
    let fx = fixture().await;
    run(&fx, "alpha.book.put", json!({"id": "b1", "title": "Dune"}))
        .await
        .unwrap();
    let again = run(&fx, "alpha.book.put", json!({"id": "b1", "title": "Dup"})).await;
    assert!(again.is_err(), "{again:?}");
    let peek = run(&fx, "alpha.book.peek", json!({"id": "b1"}))
        .await
        .unwrap();
    assert_eq!(peek["result"]["books"]["rows"][0]["title"], "Dune");
    assert_eq!(peek["result"]["file"], json!({ "body": "Dune" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn another_extension_cannot_reach_the_file_and_nobody_else_can_write_it() {
    let fx = fixture().await;
    run(&fx, "alpha.book.put", json!({"id": "b1", "title": "Dune"}))
        .await
        .unwrap();
    // beta has a file of its own, with only its seed.
    let beta = run(&fx, "beta.book.peek", json!({"id": "b1"})).await;
    assert!(beta.is_err(), "beta has no shelf-beta/b1.md: {beta:?}");
    run(&fx, "beta.book.put", json!({"id": "x", "title": "X"}))
        .await
        .unwrap();
    assert!(store_file(&fx, "beta").exists());
    let alpha = run(&fx, "alpha.book.peek", json!({"id": "x"})).await;
    assert!(alpha.is_err(), "alpha's file is not beta's: {alpha:?}");
    // beta's folder is not alpha's to write or read.
    let other = run(&fx, "alpha.book.escape", json!({"path": "shelf-beta/x.md"})).await;
    let message = format!("{other:?}");
    assert!(
        message.contains("shelf-beta/x.md") && message.contains("shelf-alpha (write)"),
        "{message}"
    );
    // A person running the write command directly is refused: it has no
    // extension to write as.
    for (name, input) in [
        (
            "oxplow.extension_store.write",
            json!({"batch": [{"sql": "DELETE FROM book"}]}),
        ),
        (
            "oxplow.extension_files.write",
            json!({"path": "shelf-alpha/z.md", "body": "z"}),
        ),
    ] {
        let direct = fx.svc.commands.run(&Actor::Human, name, input, false).await;
        assert!(
            matches!(direct, Err(CommandError::Denied { .. })),
            "{name}: {direct:?}"
        );
    }
    assert!(!fx._dir.path().join("shelf-alpha/z.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_read_service_refuses_a_statement_that_writes() {
    let fx = fixture().await;
    let result = run(&fx, "alpha.book.mutate", json!({})).await;
    assert!(result.is_err(), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_outside_the_declared_folders_is_refused_naming_the_folders() {
    let fx = fixture().await;
    for path in [
        "/etc/passwd",
        "shelf-alpha/../secret",
        "src/main.rs",
        "docs/../x",
    ] {
        let result = run(&fx, "alpha.book.escape", json!({ "path": path })).await;
        let message = format!("{result:?}");
        assert!(result.is_err(), "{path}");
        assert!(
            message.contains(&format!("`{path}`"))
                && message.contains("shelf-alpha (write), docs (read)"),
            "{message}"
        );
    }
    // A read-only folder takes no write.
    let write = run(&fx, "alpha.book.put_doc", json!({})).await;
    assert!(format!("{write:?}").contains("read-only"), "{write:?}");
    assert!(!fx._dir.path().join("docs/a.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_keeps_the_file_and_removing_deletes_it() {
    let fx = fixture().await;
    run(&fx, "alpha.book.put", json!({"id": "b1", "title": "Dune"}))
        .await
        .unwrap();
    write(
        fx._dir.path(),
        ".oxplow/local.yaml",
        "oxplow.extensions.alpha.enabled: false\n",
    );
    reconcile(&fx).await;
    assert!(fx.svc.commands.spec("alpha.book.peek").is_none());
    assert!(
        store_file(&fx, "alpha").exists(),
        "a disable keeps the file"
    );

    std::fs::remove_file(fx._dir.path().join(".oxplow/local.yaml")).unwrap();
    reconcile(&fx).await;
    let peek: Value = run(&fx, "alpha.book.peek", json!({"id": "b1"}))
        .await
        .unwrap();
    assert_eq!(peek["result"]["books"]["rows"][0]["title"], "Dune");

    crate::extension::extension_deploy::remove(&fx.svc, "alpha")
        .await
        .unwrap();
    assert!(
        !store_file(&fx, "alpha").exists(),
        "an uninstall removes it"
    );
}
