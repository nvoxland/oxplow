use super::*;
use crate::Format;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// The tally extension: a starlark collector of `thing`s, a dataset and a
/// page over them, a command noting one, a README and questions, and an
/// intent example per kind, each with its fixture.
fn tally(root: &Path, collector_expect: &str, command_expect: &str) {
    write(
            root,
            ".oxplow/extensions/tally/extension.yaml",
            &format!(
                "manifest: 2
name: tally
intent:
  purpose: Count things.
  examples:
    - {{ name: labelled, input: {{ page: labelled }}, expect: no labelled things yet }}
    - {{ name: things, input: {{ collector: things }}, expect: one thing per input row }}
    - {{ name: shell, input: {{ collector: shell }}, expect: whatever the shell says }}
permissions: [sql.read, ai.compute, process.run]
collectors:
  - id: things
    runtime: starlark
    entry: collectors/things.star
    input: \"SELECT uri FROM v_work_item\"
    entities:
      - {{ name: thing, key: id, columns: {{ id: int, label: text }} }}
  - id: shell
    runtime: exec
    entry: collectors/shell.sh
    entities:
      - {{ name: line, key: n, columns: {{ n: int }} }}
datasets:
  - name: labelled
    version: 1
    description: Labelled things.
    fields:
      - {{ name: id, type: INTEGER, doc: The thing. }}
      - {{ name: label, type: TEXT, doc: Its label. }}
commands:
  - name: work.note
    summary: Note a work item.
    input_schema: {{ type: object, required: [uri], properties: {{ uri: {{ type: string }} }} }}
    entry: handlers/note.star
    examples:
      - {{ name: happy, input: {{ uri: \"work_item:oxplow:tsk1\" }}, expect_commands: {command_expect} }}
"
            ),
        );
    write(
            root,
            ".oxplow/extensions/tally/collectors/things.star",
            "def transform(x):\n    return {\"entities\": {\"thing\": [{\"id\": r[\"id\"], \"label\": \"t\"} for r in x[\"rows\"]]}}\n",
        );
    write(
        root,
        ".oxplow/extensions/tally/collectors/shell.sh",
        "#!/bin/sh\necho '{}'\n",
    );
    write(
        root,
        ".oxplow/extensions/tally/datasets/labelled.sql",
        "SELECT id, label FROM ref('thing') WHERE label IS NOT NULL\n",
    );
    write(
            root,
            ".oxplow/extensions/tally/handlers/note.star",
            "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.work_item.comment\", \"input\": {\"uri\": x[\"input\"][\"uri\"], \"body\": \"noted\"}}]}\n",
        );
    write(
            root,
            ".oxplow/extensions/tally/pages/labelled.yaml",
            "title: Labelled\nloaders:\n  labelled: { sql: \"SELECT id, label FROM v_tally_labelled\" }\n\
             view: { viz: table, from: labelled }\n",
        );
    write(
            root,
            ".oxplow/extensions/tally/fixtures/labelled.yaml",
            "when: { page: labelled }\nexpect: { loaders: { labelled: { columns: [id, label], rows: 0 } } }\n",
        );
    write(
            root,
            ".oxplow/extensions/tally/fixtures/things.yaml",
            &format!(
                "when: {{ collector: things, rows: [{{ id: 1 }}, {{ id: 2 }}] }}\nexpect: {collector_expect}\n"
            ),
        );
    write(
        root,
        ".oxplow/extensions/tally/fixtures/shell.yaml",
        "when: { collector: shell }\nexpect: { entities: { line: $any } }\n",
    );
    write(
        root,
        ".oxplow/extensions/tally/README.md",
        "Read `v_tally_labelled`; note a work item with `tally.work.note`.\n",
    );
    write(
            root,
            ".oxplow/extensions/tally/questions.yaml",
            "- question: Which things are labelled?\n  skill: README.md\n  reaches:\n    sql: SELECT id, label FROM v_tally_labelled\n  shape: { columns: [id, label] }\n- question: Note a work item.\n  skill: README.md\n  reaches:\n    command: tally.work.note\n    input: { uri: \"work_item:oxplow:tsk1\" }\n",
        );
}

/// A collector that calls a model is a program a person approves; the
/// throwaway approves it as a person would, and its example runs
/// against the fixture model — so it never spends a key.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_a_collector_that_calls_a_model_against_the_fixture_model() {
    let dir = tempfile::tempdir().unwrap();
    tally(
        dir.path(),
        "{ entities: { thing: 2 } }",
        "[oxplow.work_item.comment]",
    );
    write(
            dir.path(),
            ".oxplow/extensions/tally/collectors/things.star",
            "def transform(x):\n    return {\"entities\": {\"thing\": [{\"id\": r[\"id\"], \"label\": ai_summarize(\"t\")} for r in x[\"rows\"]]}}\n",
        );
    let report = test_extension(dir.path(), "tally", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "example things"),
        "{:?}",
        report.ran
    );
    let warnings = report.warning_lines().join("\n");
    assert!(!warnings.contains("calls a model"), "{warnings}");
}

/// P7.C6: `extension test` runs every kind's examples on a throwaway
/// oxplow — a page's loader rows, a starlark collector's typed entities from
/// its fixture rows, a command's composition against the real
/// registry — and its questions may name its own commands. An exec
/// collector's example isn't run (it needs a person's approval).
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_every_kinds_examples() {
    let dir = tempfile::tempdir().unwrap();
    tally(
        dir.path(),
        "{ entities: { thing: 2 } }",
        "[oxplow.work_item.comment]",
    );
    let report = test_extension(dir.path(), "tally", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    for ran in ["check", "example labelled", "example things", "questions"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
    let warnings = report.warning_lines().join("\n");
    assert!(
        warnings.contains(
            "fixtures/shell.yaml: example `shell` runs collector `shell`, which runs a program"
        ),
        "{warnings}"
    );

    let dir = tempfile::tempdir().unwrap();
    tally(
        dir.path(),
        "{ entities: { thing: 3 } }",
        "[oxplow.work_item.comment]",
    );
    let report = test_extension(dir.path(), "tally", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors
            .contains("fixtures/things.yaml:2: collector `things` returned 2 at `/entities/thing`"),
        "{errors}"
    );

    // A command's example is checked against the throwaway's registry.
    let dir = tempfile::tempdir().unwrap();
    tally(
        dir.path(),
        "{ entities: { thing: 2 } }",
        "[oxplow.work_item.transition]",
    );
    let report = test_extension(dir.path(), "tally", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
            errors.contains("example `happy`: composed [oxplow.work_item.comment] but `expect_commands` is [oxplow.work_item.transition]"),
            "{errors}"
        );
}

/// A producer exporting `items` at `version`, and a consumer reading it
/// through an optional import of version 1.
fn producer_and_consumer(root: &Path, version: u32) {
    write(
            root,
            ".oxplow/extensions/maker/extension.yaml",
            &format!(
                "manifest: 2\nname: maker\nintent:\n  purpose: Make items.\ndatasets:\n  - name: items\n    version: {version}\n    description: Items.\n    fields:\n      - {{ name: uri, type: TEXT, doc: A work item. }}\nexports:\n  - {{ contribution: oxplow:oxplow/dataset?dataset=maker/items, version: {version}, visibility: stable }}\n"
            ),
        );
    write(
        root,
        ".oxplow/extensions/maker/datasets/items.sql",
        "SELECT uri FROM ref('work_item')\n",
    );
    write(
            root,
            ".oxplow/extensions/reader/extension.yaml",
            "manifest: 2\nname: reader\nintent:\n  purpose: Read the maker's items.\nimports:\n  - { id: items, contribution: oxplow:oxplow/dataset?dataset=maker/items, version: 1, optional: true }\n",
        );
    write(
        root,
        ".oxplow/extensions/reader/pages/seen.yaml",
        "title: Seen\nloaders:\n  seen: { sql: \"SELECT uri FROM import_ref('items')\" }\n\
             view: { viz: table, from: seen }\n",
    );
}

/// `extension test` says how each import resolved: available, or why
/// not (the producer disabled, or exporting another version).
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_reports_how_each_import_resolved() {
    let dir = tempfile::tempdir().unwrap();
    producer_and_consumer(dir.path(), 1);
    let report = test_extension(dir.path(), "reader", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert_eq!(
        report.imports,
        ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: available"]
    );
    assert!(render(&report, crate::Format::Text)
        .contains("import `items` → oxplow:oxplow/dataset?dataset=maker/items v1: available\n"));
    let checked = crate::check(
        dir.path(),
        "reader",
        &oxplow_app::extension::extension_catalog::ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(crate::render_findings(&checked, crate::Format::Text)
        .contains("import `items` → oxplow:oxplow/dataset?dataset=maker/items v1: available\n"));

    let dir = tempfile::tempdir().unwrap();
    producer_and_consumer(dir.path(), 2);
    let report = test_extension(dir.path(), "reader", false).await.unwrap();
    assert_eq!(
            report.imports,
            ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: unavailable (maker exports v2)"]
        );

    // A person's own disable doesn't change what a test runs with.
    let dir = tempfile::tempdir().unwrap();
    producer_and_consumer(dir.path(), 1);
    write(
        dir.path(),
        ".oxplow/personal.yaml",
        "disableExtensions: [maker]\n",
    );
    let report = test_extension(dir.path(), "reader", false).await.unwrap();
    assert_eq!(
        report.imports,
        ["`items` → oxplow:oxplow/dataset?dataset=maker/items v1: available"]
    );
}

/// A read command's example checks what it answers (`expect: {
/// result }`), only the keys the example names, beside or instead of
/// what it composes; a different answer is a finding.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_commands_example_checks_its_result() {
    for (want, ok) in [("{ count: 2, of: things }", true), ("{ count: 3 }", false)] {
        let dir = tempfile::tempdir().unwrap();
        tally(
            dir.path(),
            "{ entities: { thing: 2 } }",
            "[oxplow.work_item.comment]",
        );
        let manifest = dir.path().join(".oxplow/extensions/tally/extension.yaml");
        let yaml = std::fs::read_to_string(&manifest).unwrap().replace(
                "    - {{ name: shell,".replace("{{", "{").as_str(),
                "    - { name: counted, input: { command: tally.things.count }, expect: the things counted }\n    - { name: shell,",
            ) + "  - name: things.count\n    summary: How many things there are.\n    entry: handlers/count.star\n    needs: [sql.read]\n    access: read\n    input_schema: { type: object }\n";
        std::fs::write(&manifest, yaml).unwrap();
        write(
                dir.path(),
                ".oxplow/extensions/tally/handlers/count.star",
                "def transform(x):\n    rows = scope(\"sql.read\", {\"sql\": \"SELECT id FROM v_tally_labelled\"})\n    return {\"commands\": [], \"result\": {\"count\": len(rows), \"of\": \"things\"}}\n",
            );
        write(
                dir.path(),
                ".oxplow/extensions/tally/fixtures/counted.yaml",
                &format!(
                    "when: {{ command: tally.things.count, input: {{}}, answers: {{ sql.read: [[{{ id: 1 }}, {{ id: 2 }}]] }} }}\nexpect: {{ result: {want} }}\n"
                ),
            );
        let report = test_extension(dir.path(), "tally", false).await.unwrap();
        let errors = report.error_lines().join("\n");
        if ok {
            assert_eq!(errors, "", "{want}");
            assert!(
                report.ran.iter().any(|r| r == "example counted"),
                "{:?}",
                report.ran
            );
        } else {
            assert!(
                    errors.contains("fixtures/counted.yaml:2: command `tally.things.count` answered 2 at `/result/count`, the example expects 3"),
                    "{errors}"
                );
        }
    }
}

/// An extension with one incremental dataset over page visits, watermarked
/// on `watermark`, and its `before` / `after` fixture.
/// The `visits` extension: an incremental dataset keyed and watermarked
/// on `id`, with a fixture whose `before` visit has id `before` and whose
/// later `after` visit has id `after`.
fn visits(root: &Path, before: i64, after: i64) {
    write(
        root,
        ".oxplow/extensions/visits/extension.yaml",
        "manifest: 2
name: visits
intent:
  purpose: Long visits.
  origin: oxplow:oxplow/thread?id=thr1
  examples: []
datasets:
  - name: long_visits
    version: 1
    description: Visits and how long they lasted.
    key: [id]
    materialize: { incremental: id }
    fields:
      - { name: id, type: INTEGER, doc: The visit. }
      - { name: duration_ms, type: INTEGER, doc: How long it lasted. }
",
    );
    write(
        root,
        ".oxplow/extensions/visits/datasets/long_visits.sql",
        "SELECT id, duration_ms FROM ref('page_visit') WHERE duration_ms IS NOT NULL\n",
    );
    write(
            root,
            ".oxplow/extensions/visits/fixtures/dataset-long_visits.yaml",
            &format!(
                "before:
  page_visit:
    - {{ id: {before}, page_kind: task, page_id: \"task:1\", visited_at: \"2026-01-01T00:00:00.000000Z\", duration_ms: 50 }}
after:
  page_visit:
    - {{ id: {after}, page_kind: task, page_id: \"task:2\", visited_at: \"2026-01-01T00:00:01.000000Z\", duration_ms: 10 }}
"
            ),
        );
}

/// P8.B5: each incremental dataset is appended to on its fixture's
/// `after` rows and checked against a full refill — a key that doesn't
/// grow with arrivals (a row lands below the watermark, unseen) fails
/// at the dataset's line; one that does passes.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_checks_incremental_datasets_against_a_full_refill() {
    let dir = tempfile::tempdir().unwrap();
    visits(dir.path(), 1, 2);
    let report = test_extension(dir.path(), "visits", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "incremental long_visits"),
        "{:?}",
        report.ran
    );

    let dir = tempfile::tempdir().unwrap();
    // A visit that arrives with a smaller id than one already appended.
    visits(dir.path(), 2, 1);
    let report = test_extension(dir.path(), "visits", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors.contains(
            ".oxplow/extensions/visits/extension.yaml:8: incremental dataset `long_visits`"
        ) && errors.contains("1 row(s) a full refill holds"),
        "{errors}"
    );
}

/// P8.D12: a scaffolded effect checks and tests clean; a fixture that
/// expects other commands is a finding at its `file:line`.
#[tokio::test(flavor = "multi_thread")]
async fn a_scaffolded_effect_tests_clean_and_a_mismatch_names_its_fixture() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
    assert!(
        report.ran.contains(&"example basic".to_string()),
        "{:?}",
        report.ran
    );

    let fixture = dir
        .path()
        .join(".oxplow/extensions/notes/fixtures/basic.yaml");
    let text = std::fs::read_to_string(&fixture).unwrap();
    std::fs::write(
        &fixture,
        text.replace(
            "commands: [oxplow.work_item.comment]",
            "commands: [oxplow.work_item.update]",
        ),
    )
    .unwrap();
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors
            .contains(".oxplow/extensions/notes/fixtures/basic.yaml:4: effect `on-done` composed"),
        "{errors}"
    );
}

/// An event fixture names the commands an effect ran, not the one
/// sequence that carried them.
#[tokio::test(flavor = "multi_thread")]
async fn an_event_fixture_lists_the_commands_an_effect_ran_not_their_sequence() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: T }, bind: t }]
when:
  event: { type: work_item.state_changed, payload: { work_item: $t.uri, to: done } }
expect:
  commands: [oxplow.work_item.comment]
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// A command fixture lists what reacted to it by the commands an effect
/// ran, not the sequence that carried them.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_fixture_lists_the_commands_an_effect_ran_not_their_sequence() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: T, state: in_progress }, bind: t }]
when: { command: oxplow.work_item.transition, input: { uri: $t.uri, to: done } }
expect:
  ok: true
  commands: [oxplow.work_item.comment]
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { turn }` makes a turn the way hooks report one: its
/// prompt, its tool calls (a failed one, one naming a file), its
/// answer, and the file it wrote.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_turn_through_the_hook_ingest() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
        dir.path(),
        ".oxplow/extensions/notes/fixtures/basic.yaml",
        "given:
  - observe:
      turn:
        prompt: Add CSV export
        tools:
          - { name: Edit, kind: edit, paths: [src/report.rs], ok: true }
          - { name: Bash, kind: shell, command: cargo test, detail: cargo test, ok: false }
        answer: Done; used the csv crate.
when:
  sql: SELECT tool, path, ok FROM v_tool_call ORDER BY id
expect:
  rows:
    - { tool: Edit, path: src/report.rs, ok: 1 }
    - { tool: Bash, ok: 0 }
",
    );
    write(
        dir.path(),
        ".oxplow/extensions/notes/fixtures/answer.yaml",
        "given:
  - observe: { turn: { prompt: Add CSV export, answer: Done; used the csv crate. } }
when:
  sql: SELECT prompt, answer FROM v_agent_turn
expect:
  rows: [{ prompt: Add CSV export, answer: Done; used the csv crate. }]
",
    );
    let manifest = dir.path().join(".oxplow/extensions/notes/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replacen(
            "  examples:\n",
            "  examples:\n    - { name: answer, expect: the turn's answer }\n",
            1,
        ),
    )
    .unwrap();
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
    assert!(
        report.ran.contains(&"example answer".to_string()),
        "{:?}",
        report.ran
    );
}

/// `observe: { event }` logs an event and lets the effects react, so a
/// scenario reads what they did.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_an_event_the_effects_react_to() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Effect, "notes", None).unwrap();
    write(
        dir.path(),
        ".oxplow/extensions/notes/fixtures/basic.yaml",
        "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: T }, bind: t }
  - observe: { event: { type: work_item.state_changed, payload: { work_item: $t.uri, to: done } } }
when: { sql: SELECT count(*) AS n FROM v_work_item_comment }
expect: { rows: [{ n: 1 }] }
",
    );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { commit }` stores a commit the way the git indexer does.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { commit: { sha: c1, at: \"2026-01-01T00:00:00Z\", subject: first, files: [{ path: a.rs, status: A, additions: 1 }] } }
  - observe: { commit: { sha: c2, at: \"2026-01-02T00:00:00Z\", subject: second, files: [{ path: a.rs, additions: 2, deletions: 1 }] } }
when:
  sql: SELECT c.subject, f.status, f.additions FROM v_commit c JOIN v_commit_file f ON f.sha = c.sha WHERE f.path = 'a.rs' ORDER BY c.committed_at DESC
expect:
  rows:
    - { subject: second, status: M, additions: 2 }
    - { subject: first, status: A, additions: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { change }` stores a change's analysis as the change
/// analysis does: the files and functions the datasets read.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_changes_analysis() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe:
      change:
        kind: commit
        target: abc
        files:
          - { path: src/hot.rs, status: modified, additions: 10, deletions: 2 }
          - { path: src/new.rs, status: added, additions: 3, is_test: true }
        functions:
          - { path: src/hot.rs, symbol: grow, status: modified, body_changed: true, length: 20, complexity: 17, complexity_delta: 3 }
when:
  sql: SELECT f.path, f.additions, f.is_test, (SELECT count(*) FROM v_change_function g WHERE g.change_id = f.change_id AND g.complexity = 17) AS fns FROM v_change_file f JOIN v_change c ON c.id = f.change_id WHERE c.kind = 'commit' AND c.target = 'abc' ORDER BY f.path
expect:
  rows:
    - { path: src/hot.rs, additions: 10, is_test: 0, fns: 1 }
    - { path: src/new.rs, additions: 3, is_test: 1, fns: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { change }` of an effort's change takes the effort's id.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_an_efforts_change() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe: { change: { kind: effort, target: eff1, files: [{ path: src/a.rs, additions: 10, deletions: 4 }] } }
when:
  sql: SELECT c.kind, c.target, f.deletions FROM v_change c JOIN v_change_file f ON f.change_id = c.id
expect:
  rows:
    - { kind: effort, target: '1', deletions: 4 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { test_run }` records a run's cases as a run recorder does:
/// a capture of the effort with a fact per case.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_test_run() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe: { test_run: { effort: eff1, at: \"2026-01-01T00:00:00Z\", cases: [{ name: a, status: passed }, { name: b, status: failed }] } }
  - observe: { test_run: { effort: eff1, at: \"2026-01-01T00:01:00Z\", asserted_failed: 2 } }
when:
  sql: SELECT dims_json, count(*) AS n FROM v_fact WHERE measure_key = 'oxplow.test_case' GROUP BY dims_json ORDER BY n
expect:
  rows:
    - { dims_json: '{\"oxplow.status\":\"passed\"}', n: 1 }
    - { dims_json: '{\"oxplow.status\":\"failed\"}', n: 3 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { snapshot }` appends to the stream's snapshot log: each
/// take's parent is the one before it.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_snapshot_log() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { snapshot: { trigger: startup, elapsed_ms: 5, files: [{ path: a.txt, hash: h1 }] } }
  - observe: { snapshot: { trigger: quiet, elapsed_ms: 900, budget_ms: 500, files: [{ path: a.txt, hash: h2 }] } }
when:
  sql: SELECT trigger, over_budget, file_count, parent_snapshot_id IS NULL AS root FROM v_snapshot_op ORDER BY seq DESC
expect:
  rows:
    - { trigger: quiet, over_budget: 1, file_count: 1, root: 0 }
    - { trigger: startup, over_budget: 0, file_count: 1, root: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { diagnostics }` is what a language server reported for a
/// file; `observe: { symbols }` the definitions the index read from it.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_diagnostics_and_symbols() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { diagnostics: { path: a.rs, items: [{ severity: error, message: broken, line: 3, col: 1 }, { severity: warning, message: unused, line: 9 }] } }
  - observe: { symbols: { path: src/lib.rs, language: rust, items: [{ name: main, kind: fn, line: 1 }, { name: helper, kind: fn, container: main, line: 5 }] } }
when:
  sql: SELECT 'd' AS k, path AS a, severity AS b FROM v_diagnostic UNION ALL SELECT 's', path, name FROM v_symbol ORDER BY 1, 3
expect:
  rows:
    - { k: d, a: a.rs, b: error }
    - { k: d, a: a.rs, b: warning }
    - { k: s, a: src/lib.rs, b: helper }
    - { k: s, a: src/lib.rs, b: main }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { facts }` records findings of a measure, as a collector's
/// capture does.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_facts() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { facts: { measure: oxplow.test_case, items: [{ path: src/a.rs, line: 3, rule: bug }, { path: src/b.rs, line: 9, rule: cleanup, value: 2, dims: { oxplow.status: failed } }] } }
when:
  sql: SELECT path, line, rule, value, dims_json LIKE '%failed%' AS failed FROM v_fact WHERE measure_key = 'oxplow.test_case' ORDER BY path
expect:
  rows:
    - { path: src/a.rs, line: 3, rule: bug, value: 1.0, failed: null }
    - { path: src/b.rs, line: 9, rule: cleanup, value: 2.0, failed: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { visit }` is a page the person opened, as the shell
/// records it.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_page_visits() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { visit: { page: \"oxplow:oxplow/page?page=problems\", kind: problems, label: Problems, duration_ms: 1200 } }
  - observe: { visit: { page: \"oxplow:oxplow/page?page=problems\", kind: problems } }
  - observe: { visit: { page: \"oxplow:oxplow/page?page=symbols\", kind: symbols, thread: thr1 } }
when:
  sql: SELECT page_kind, page_id, label, duration_ms, thread_id FROM v_page_visit ORDER BY id
expect:
  rows:
    - { page_kind: problems, page_id: \"oxplow:oxplow/page?page=problems\", label: Problems, duration_ms: 1200 }
    - { page_kind: problems }
    - { page_kind: symbols, thread_id: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// A test's oxplow has only what is declared on: the required
/// foundation, the extension tested and what it requires. Oxplow's own
/// pages run complete without the default extensions adding to them.
#[tokio::test(flavor = "multi_thread")]
async fn a_foundation_fixture_runs_without_bundled() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    let host = Host::start(
        dir.path(),
        "oxplow-foundation",
        &[],
        Arc::new(|_: &str| None),
        false,
    )
    .await
    .unwrap();
    observe_commit(
            &host,
            &json!({ "sha": "c1", "at": "2026-01-01T00:00:00Z", "subject": "first", "files": [{ "path": "a.rs" }] }),
        )
        .await
        .unwrap();
    let run = run_page(
        &host,
        oxplow_app::pages::runner::RunTarget::Page("oxplow-foundation/git-history".into()),
        &json!({ "stream_id": 1, "head_sha": "c1" }),
        &oxplow_app::pages::spec::ViewContext::default(),
    )
    .await
    .unwrap();
    fn walk<'a>(
        n: &'a oxplow_app::pages::runner::RunNode,
        out: &mut Vec<&'a oxplow_app::pages::runner::RunNode>,
    ) {
        out.push(n);
        for c in &n.children {
            walk(c, out);
        }
    }
    let mut nodes = Vec::new();
    walk(&run.tree, &mut nodes);
    assert!(!nodes.is_empty());
    for n in nodes {
        assert_eq!(n.replaced_by, None, "{}", n.id);
        assert!(!n.id.starts_with("oxplow-bundled."), "{}", n.id);
    }
}

/// A fixture's `requires` turns on extensions beyond the one tested
/// (and what they require): what only oxplow-bundled adds (its pages)
/// is there when a fixture asks for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_requires_the_extensions_it_needs() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    let fixture = |requires: &str| {
        format!(
            "{requires}given:
when: {{ sql: SELECT count(*) > 0 AS here FROM v_page WHERE extension = 'oxplow-bundled' }}
expect: {{ rows: [{{ here: 1 }}] }}
"
        )
    };
    write(
        dir.path(),
        ".oxplow/extensions/notes/fixtures/basic.yaml",
        &fixture("requires: [oxplow-bundled]\n"),
    );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    // Without it its pages aren't there.
    std::fs::write(
        dir.path()
            .join(".oxplow/extensions/notes/fixtures/basic.yaml"),
        fixture(""),
    )
    .unwrap();
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(!report.errors.is_empty());
    // A name that isn't an extension here is a finding.
    write(
        dir.path(),
        ".oxplow/extensions/notes/fixtures/basic.yaml",
        &fixture("requires: [no-such-extension]\n"),
    );
    let errors = test_extension(dir.path(), "notes", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/basic.yaml: `requires` names `no-such-extension`"),
        "{errors}"
    );
}

/// `observe: { refs }` is what the git indexer read of the repository's
/// branches, tags and the stream's head; `snapshot` takes a `revision`.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_git_refs_and_a_snapshots_revision() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { commit: { sha: c1, at: \"2026-01-01T00:00:00Z\", subject: first } }
  - observe: { refs: { branches: [{ name: main, head: c1, stream: 1 }, { name: side, head: c1 }], tags: [{ name: v1, sha: c1 }], heads: [{ stream: 1, sha: c1 }] } }
  - observe: { snapshot: { trigger: quiet, revision: \"git:c1\", files: [{ path: a.txt, hash: h1 }] } }
when:
  sql: SELECT (SELECT group_concat(name) FROM (SELECT name FROM v_branch ORDER BY name)) AS branches, (SELECT name FROM v_tag) AS tag, (SELECT head_sha FROM v_stream) AS head, (SELECT revision FROM v_snapshot) AS revision
expect:
  rows:
    - { branches: \"main,side\", tag: v1, head: c1, revision: \"git:c1\" }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { run }` and `{ health }` are what the extension host
/// records of a contribution's run and its failures; `token_usage`,
/// `hint` and `nudge` what a harness and the hints record.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_runs_health_usage_and_hints() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - observe: { run: { extension: acme, contribution: pull, kind: collector, outcome: failed, days_ago: 2, cpu_ms: 30, ai_calls: 2 } }
  - observe: { run: { extension: acme, contribution: pull, kind: collector, outcome: ok } }
  - observe: { health: { extension: acme, contribution: pull, kind: collector, disable: boom } }
  - observe: { token_usage: { prompt: fix it, input_tokens: 1999, output_tokens: 1 } }
  - observe: { hint: { hint: x/never, evaluated: 3 } }
  - observe: { nudge: { kind: x/spoke, message: m } }
when:
  sql: SELECT (SELECT group_concat(outcome || ':' || cpu_ms || ':' || ai_calls, ',') FROM (SELECT * FROM v_extension_run ORDER BY started_at)) AS runs, (SELECT state || ':' || reason FROM v_contribution_health) AS health, (SELECT input_tokens FROM v_token_usage) AS tokens, (SELECT evaluated FROM v_hint_stat WHERE hint = 'x/never') AS evaluated, (SELECT count(*) FROM v_agent_nudge WHERE kind = 'x/spoke') AS nudges
expect:
  rows:
    - { runs: \"failed:30:2,ok:0:0\", health: \"disabled:boom\", tokens: 1999, evaluated: 3, nudges: 1 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// A turn is observed on a thread of its own, in a session of its own,
/// with a subagent's hand-back (no prompt) and a call as it starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_a_turn_on_a_thread_and_session_with_handbacks() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "given:
  - { command: oxplow.thread.create, input: { stream: str1, title: Beta }, bind: beta }
  - { command: oxplow.agent_session.open, input: { thread: $beta.id, harness: codex }, bind: ses }
  - observe: { turn: { thread: $beta.id, session: $ses.id, prompt: first ask, answer: one, tools: [{ name: ExitPlanMode, kind: plan, plan: { text: \"1. First\" }, pre: true }] } }
  - observe: { turn: { thread: $beta.id, session: $ses.id, prompt: null, subagent: { id: a1, kind: Explore }, answer: report } }
when:
  sql: SELECT t.prompt, t.answer, th.title FROM v_agent_turn t JOIN v_thread th ON th.id = t.thread_id WHERE th.title = 'Beta' ORDER BY t.id
expect:
  rows:
    - { prompt: first ask, answer: one, title: Beta }
    - { prompt: \"\", answer: report, title: Beta }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// `observe: { evidence }` sets the observations an effort's page
/// reads.
#[tokio::test(flavor = "multi_thread")]
async fn a_given_observes_an_efforts_evidence() {
    let dir = tempfile::tempdir().unwrap();
    crate::scaffold(dir.path(), crate::Kind::Command, "notes", None).unwrap();
    write(
            dir.path(),
            ".oxplow/extensions/notes/fixtures/basic.yaml",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.effort.open, input: { thread: thr1 } }
  - observe:
      evidence:
        effort: eff1
        observations:
          - { kind: test-run, payload: { passed: 4, failed: 1 } }
          - { kind: diff-coverage, metric_value: 79.5, payload: { summaryPct: 79.5 } }
when:
  sql: SELECT kind, metric_value, provenance FROM v_effort_observation WHERE effort_id = 1 ORDER BY seq
expect:
  rows:
    - { kind: test-run, metric_value: null, provenance: observed }
    - { kind: diff-coverage, metric_value: 79.5 }
",
        );
    let report = test_extension(dir.path(), "notes", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
}

/// A policy that opens an item's effort when it starts on a thread,
/// and closes its open efforts (read through `scope`) when it ends.
const TIDY: &str = r#"
def transform(x):
    e = x["event"]
    if e["type"] != "work_item.state_changed":
        return {"skip": "not an item's move"}
    item = e["payload"]["work_item"]
    to = e["payload"]["to"]
    thread = e["anchors"].get("thread_id")
    if to == "in_progress" and thread != None:
        return {"commands": [{"name": "oxplow.effort.open",
                              "input": {"thread": thread, "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {"sql": "SELECT uri FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
                                  "params": {"w": item}})
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": uri.param(r["uri"], "id"), "reason": "switch"}}
                             for r in rows]}
    return {"skip": "a move that changes no effort"}
"#;

/// The acme extension: `script` as the effort policy `tidy`, with an
/// example per reaction — a checkpoint (`basic`, expecting
/// `basic_expect`), a start on a thread, a finish whose read is
/// answered.
fn policy(root: &Path, script: &str, basic_expect: &str) {
    write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: Open and close efforts as items start and finish.
  examples:
    - { name: basic, expect: nothing to do }
    - { name: start, expect: its effort opens }
    - { name: finish, expect: its effort closes }
permissions: [sql.read, commands:oxplow.effort.*]
implementations:
  - { capability: effort_policy, id: tidy, title: Tidy, entry: policies/tidy.star, needs: [sql.read] }
",
        );
    write(root, ".oxplow/extensions/acme/policies/tidy.star", script);
    write(
            root,
            ".oxplow/extensions/acme/fixtures/basic.yaml",
            &format!(
                "when: {{ implementation: tidy, event: {{ type: thread.checkpoint, payload: {{ thread: thr1, turn: trn1, reason: turn_end, snapshot: 1, changed: false, writing_tools: 0 }} }} }}\nexpect: {basic_expect}\n"
            ),
        );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/start.yaml",
            "requires: [oxplow-bundled]\nwhen: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: in_progress }, anchors: { thread_id: thr1 } } }\nexpect: { commands: [oxplow.effort.open] }\n",
        );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/finish.yaml",
            "requires: [oxplow-bundled]\nwhen: { implementation: tidy, event: { type: work_item.state_changed, payload: { work_item: \"work_item:oxplow:tsk1\", to: done } }, answers: { sql.read: [[{ uri: \"oxplow:oxplow/effort?id=eff3\" }]] } }\nexpect: { commands: [oxplow.effort.close] }\n",
        );
}

/// A scripted effort policy's examples dry-run its script (a read
/// answered by the fixture), and the effort-policy suite runs over it
/// in a throwaway host.
#[tokio::test(flavor = "multi_thread")]
async fn a_policy_script_runs_its_examples_and_the_suite() {
    let dir = tempfile::tempdir().unwrap();
    policy(dir.path(), TIDY, "{ skip: $any }");
    let report = test_extension(dir.path(), "acme", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
    for ran in [
        "example basic",
        "example start",
        "example finish",
        "effort_policy suite",
    ] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
}

/// A policy example that doesn't match what the script answers is a
/// finding at its fixture; one naming no policy of the extension names
/// what it named.
#[tokio::test(flavor = "multi_thread")]
async fn a_policy_example_that_doesnt_match_names_its_fixture() {
    let dir = tempfile::tempdir().unwrap();
    policy(dir.path(), TIDY, "{ commands: [oxplow.effort.open] }");
    let finish = dir
        .path()
        .join(".oxplow/extensions/acme/fixtures/finish.yaml");
    let text = std::fs::read_to_string(&finish).unwrap();
    std::fs::write(
        &finish,
        text.replace("implementation: tidy", "implementation: nope"),
    )
    .unwrap();
    let report = test_extension(dir.path(), "acme", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors.contains("acme/fixtures/basic.yaml:2: effort policy `tidy` skipped"),
        "{errors}"
    );
    assert!(
        errors.contains("acme/fixtures/finish.yaml: example `finish` names effort policy `nope`"),
        "{errors}"
    );
}

/// An example's event is the event envelope the runtime would hand
/// over: what the fixture doesn't give is `null` (no made-up id or
/// seq), `source` is `fixture`; and a fixture that couldn't happen —
/// an unknown type, a version newer than registered, a malformed
/// anchor, a key the envelope doesn't have — is a finding at it.
#[tokio::test(flavor = "multi_thread")]
async fn an_example_event_is_the_envelope_and_one_that_couldnt_happen_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let shape = "def transform(x):\n    e = x[\"event\"]\n    return {\"skip\": \"%s %s %s %s %s\" % (e[\"v\"], e[\"source\"], e[\"id\"], e[\"seq\"], e[\"anchors\"][\"turn_id\"])}\n";
    policy(root, shape, "{ skip: \"2 fixture None None None\" }");
    // `start` and `finish` aren't this script's; only `basic` stays.
    for gone in ["start", "finish"] {
        std::fs::remove_file(root.join(format!(".oxplow/extensions/acme/fixtures/{gone}.yaml")))
            .unwrap();
    }
    let manifest = root.join(".oxplow/extensions/acme/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
            &manifest,
            text.replace(
                "    - { name: start, expect: its effort opens }\n    - { name: finish, expect: its effort closes }\n",
                "",
            ),
        )
        .unwrap();
    let report = test_extension(root, "acme", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(!errors.contains("fixtures/basic.yaml"), "{errors}");

    for (event, needle) in [
        (
            "{ type: acme.nope }",
            "`acme.nope` is not a registered event type",
        ),
        (
            "{ type: thread.checkpoint, v: 9, payload: {} }",
            "newer than the registered v2",
        ),
        (
            "{ type: thread.checkpoint, payload: {}, anchors: { turn_id: seven } }",
            "`anchors.turn_id`",
        ),
        (
            "{ type: thread.checkpoint, payload: {}, subjects: [] }",
            "unknown field `subjects`",
        ),
    ] {
        write(
            root,
            ".oxplow/extensions/acme/fixtures/basic.yaml",
            &format!(
                "when: {{ implementation: tidy, event: {event} }}\nexpect: {{ skip: $any }}\n"
            ),
        );
        let report = test_extension(root, "acme", false).await.unwrap();
        let errors = report.error_lines().join("\n");
        assert!(
            errors.contains("acme/fixtures/basic.yaml: example `basic`: event: ")
                && errors.contains(needle),
            "{event}: {errors}"
        );
    }
}

/// An effect example's event at an older version of the extension's
/// own type is carried to the newest through its upcast, as a logged
/// one would be.
#[tokio::test(flavor = "multi_thread")]
async fn an_effect_example_at_an_older_version_is_upcast() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: Note merges.
  examples:
    - { name: old, expect: an old merge is read at v2 }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: A merge. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: A merge by someone., upcast: merged.star }
effects:
  - { id: note, summary: Note a merge., on: [acme.merged], entry: note.star }
",
        );
    write(
        root,
        ".oxplow/extensions/acme/merged.v1.json",
        r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.v2.json",
        r#"{"type": "object", "required": ["number", "by"], "properties": {"number": {"type": "integer"}, "by": {"type": "string"}}}"#,
    );
    write(
            root,
            ".oxplow/extensions/acme/merged.star",
            "def transform(x):\n    p = dict(x[\"payload\"])\n    p[\"by\"] = \"unknown\"\n    return p\n",
        );
    write(
            root,
            ".oxplow/extensions/acme/note.star",
            "def transform(x):\n    e = x[\"event\"]\n    return {\"skip\": \"v%d by %s\" % (e[\"v\"], e[\"payload\"][\"by\"])}\n",
        );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/old.yaml",
            "when: { effect: note, event: { type: acme.merged, v: 1, payload: { number: 3 } } }\nexpect: { skip: v2 by unknown }\n",
        );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
    assert!(
        report.ran.iter().any(|r| r == "example old"),
        "{:?}",
        report.ran
    );
}

/// A policy that opens efforts and never closes one fails the suite,
/// at the manifest.
#[tokio::test(flavor = "multi_thread")]
async fn a_policy_script_that_never_closes_fails_the_suite() {
    let dir = tempfile::tempdir().unwrap();
    let opens_only = TIDY.replace(r#"if to == "done" or to == "canceled":"#, "if False:");
    policy(dir.path(), &opens_only, "{ skip: $any }");
    let report = test_extension(dir.path(), "acme", false).await.unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors.contains(
            "acme/extension.yaml: effort_policy conformance `a_finished_item_closes_its_effort`"
        ),
        "{errors}"
    );
}

/// Event-type and ref-kind fixtures: a payload checked against the
/// declared schema (and its upcast), a wikilink against the declared
/// kind; a wrong expectation is a finding at its fixture.
/// The skill's table of fixture kinds is the runner's own (`KINDS`):
/// what the runner accepts is what the skill teaches. `OXPLOW_BLESS=1`
/// writes it.
#[test]
fn the_skill_lists_every_fixture_kind_the_runner_runs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxplow-agent-text/assets/skills/oxplow-extension/references/testing.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let (start, end) = ("<!-- fixture-kinds -->\n", "<!-- /fixture-kinds -->");
    let (Some(a), Some(b)) = (text.find(start), text.find(end)) else {
        panic!("testing.md has no `{start}` … `{end}` block");
    };
    let table = kinds_table();
    if std::env::var_os("OXPLOW_BLESS").is_some() {
        let blessed = format!("{}{start}{table}{}", &text[..a], &text[b..]);
        std::fs::write(&path, blessed).unwrap();
        return;
    }
    assert_eq!(
        text[a + start.len()..b],
        table,
        "testing.md's fixture kinds are stale: regenerate them with OXPLOW_BLESS=1"
    );
}

/// One rule compares every `expect`: an object only on the keys it
/// names, a list item by item in order, `{ contains: [..] }` items a
/// list has in any order, `$any` anything, `{ $exact: v }` the whole value.
#[test]
fn one_rule_compares_what_an_example_expects() {
    let got = json!({ "state": "done", "rows": [{ "a": 1, "b": 2 }, { "a": 3, "b": 4 }] });
    let holds = |want: Value| compare(&want, &got).is_none();
    assert!(holds(json!({ "state": "done" })));
    assert!(holds(json!({ "rows": [{ "a": 1 }, { "b": 4 }] })));
    assert!(
        !holds(json!({ "rows": [{ "a": 1 }] })),
        "a list is every item"
    );
    assert!(holds(json!({ "rows": { "contains": [{ "a": 3 }] } })));
    assert!(holds(
        json!({ "rows": { "contains": [{ "b": 4 }, { "a": 1 }] } })
    ));
    assert!(!holds(json!({ "rows": { "contains": [{ "a": 5 }] } })));
    assert!(holds(json!({ "rows": "$any", "state": "$any" })));
    assert!(!holds(json!({ "missing": 1 })));
    assert!(
        !holds(json!({ "missing": "$any" })),
        "$any is a value that's there"
    );
    assert!(holds(json!({ "missing": null })));
    let no = json!({ "ok": false, "refused": "`tsk9` isn't a work item" });
    assert!(compare(&json!({ "refused": "isn't a work item" }), &no).is_none());
    assert!(compare(&json!({ "refused": "is archived" }), &no).is_some());
    assert!(holds(
        json!({ "rows": { "$exact": [{ "a": 1, "b": 2 }, { "a": 3, "b": 4 }] } })
    ));
    assert!(!holds(
        json!({ "rows": { "$exact": [{ "a": 1 }, { "a": 3 }] } })
    ));
    assert_eq!(
        compare(&json!({ "rows": [{ "a": 1 }, { "a": 9 }] }), &got),
        Some(("/rows/1/a".into(), json!(9), json!(3)))
    );
    assert!(
        holds(json!({ "rows": [{ "a": 1.0 }, { "b": 4.0 }] })),
        "a number is its value: 1 is 1.0"
    );
    assert!(!holds(json!({ "rows": [{ "a": 1.5 }, { "b": 4 }] })));
}

/// The text report names each failure and sums up what ran: the
/// examples as a count, not one by one (the JUnit form names them).
#[test]
fn the_text_report_counts_the_examples_it_ran() {
    let mut report = TestReport::part("acme", ".oxplow/extensions/acme");
    report.ran = ["check", "example first", "example second", "answers"]
        .map(String::from)
        .to_vec();
    report
        .errors
        .push(Diagnostic::error("fixture.compare", "wrong").at("fixtures/second.yaml", Some(1)));
    let text = render(&report, Format::Text);
    assert!(
        text.contains("error: .oxplow/extensions/acme/fixtures/second.yaml:1: wrong\n"),
        "{text}"
    );
    assert!(
        text.ends_with("acme: 1 error, 0 warnings; ran check, 2 examples, answers\n"),
        "{text}"
    );
    report.ran = ["check", "example first"].map(String::from).to_vec();
    assert!(
        render(&report, Format::Text).ends_with("; ran check, 1 example\n"),
        "{}",
        render(&report, Format::Text)
    );
}

/// Run with `only`, the check runs and then just the named examples;
/// a name no example has is an error naming the ones there are.
#[tokio::test(flavor = "multi_thread")]
async fn only_the_named_examples_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
            root,
            "title: Items\nloaders:\n  items: { sql: \"SELECT 1 AS n\" }\nview: { from: items, viz: table }\n",
            &[
                ("first", "when: { page: items }\nexpect: { loaders: { items: { rows: 1 } } }\n"),
                ("second", "when: { page: items }\nexpect: { loaders: { items: { rows: 5 } } }\n"),
            ],
        );
    let report = test_extension_only(root, "paged", &["first".into()], false)
        .await
        .unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert_eq!(report.ran, ["check", "example first"].map(String::from));

    let report = test_extension_only(root, "paged", &["third".into()], false)
        .await
        .unwrap();
    let errors = report.error_lines().join("\n");
    assert!(
        errors.contains("no example `third`") && errors.contains("first, second"),
        "{errors}"
    );
}

/// Every kind of fixture has `given` steps whose bound values its
/// `when` and `expect` read, and `steps`: each `{ when?, expect }` runs
/// again with its `when` over the fixture's, and a finding names its step.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_of_any_kind_has_given_and_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged }, expect: valid }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: Merged. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: Merged., upcast: merged.star }
",
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.v1.json",
        r#"{"type": "object", "required": ["number"]}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.v2.json",
        r#"{"type": "object", "required": ["pr"]}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.star",
        "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"]}\n",
    );
    let scenario = |last: &str| {
        format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Fix it }}, bind: item }}]
when: {{ event_type: acme.merged, payload: {{ pr: $item.uri }} }}
expect: {{ valid: true }}
steps:
  - when: {{ v: 1, payload: {{ number: $item.uri }} }}
    expect: {{ valid: true, upcast: {{ pr: $item.uri }} }}
  - when: {{ payload: {{ number: 1 }} }}
    expect: {{ valid: {last} }}
"
            )
    };
    write(
        root,
        ".oxplow/extensions/acme/fixtures/merged.yaml",
        &scenario("false"),
    );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    write(
        root,
        ".oxplow/extensions/acme/fixtures/merged.yaml",
        &scenario("true"),
    );
    let errors = test_extension(root, "acme", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains(".oxplow/extensions/acme/fixtures/merged.yaml: steps[1]: "),
        "{errors}"
    );
}

/// A step whose `when` names another kind — or, in a page's scenario,
/// another page — runs that instead of the fixture's own, and the
/// scenario goes on.
#[tokio::test(flavor = "multi_thread")]
async fn a_step_runs_another_kind_or_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged }, expect: valid }
    - { name: paged, input: { page: titles }, expect: titles }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.json, summary: Merged. }
",
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.json",
        r#"{"type": "object", "required": ["pr"]}"#,
    );
    write(
            root,
            ".oxplow/extensions/acme/pages/titles.yaml",
            "title: Titles\nloaders:\n  titles: { sql: \"SELECT title FROM v_work_item\" }\nview: { viz: table, from: titles }\n",
        );
    write(
            root,
            ".oxplow/extensions/acme/pages/count.yaml",
            "title: Count\nloaders:\n  n: { sql: \"SELECT COUNT(*) AS n FROM v_work_item\" }\nview: { viz: number, from: n }\n",
        );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/merged.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { event_type: acme.merged, payload: { pr: 1 } }
expect: { valid: true }
steps:
  - when: { sql: \"SELECT title FROM v_work_item\" }
    expect: { rows: [{ title: Fix it }] }
  - expect: { valid: true }
",
        );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/paged.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { page: titles }
expect: { loaders: { titles: { rows: [{ title: Fix it }] } } }
steps:
  - when: { page: count }
    expect: { loaders: { n: { rows: [{ n: 1 }] } } }
  - when: { sql: \"SELECT COUNT(*) AS n FROM v_work_item\" }
    expect: { rows: [{ n: 1 }] }
  - expect: { loaders: { titles: { rows: 1 } } }
",
        );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn event_type_and_ref_kind_fixtures_check_what_they_declare() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
            root,
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: merged, input: { event_type: acme.merged, payload: { pr: 12 } }, expect: valid }
    - { name: old, input: { event_type: acme.merged, v: 1, payload: { number: 12 } }, expect: upcast }
    - { name: link, input: { wikilink: pr:12 }, expect: the pull request }
event_types:
  types:
    - { type: acme.merged, v: 1, schema: merged.v1.json, summary: Merged. }
    - { type: acme.merged, v: 2, schema: merged.v2.json, summary: Merged., upcast: merged.star }
datasets:
  - name: prs
    version: 1
    description: PRs.
    fields:
      - { name: uri, type: \"\", doc: The ref. }
      - { name: title, type: \"\", doc: Its title. }
resources:
  - { name: pr, label: Pull request, params: { id: { required: true, pattern: '^[0-9]+$' } }, resolve: prs, wikilink: { prefix: pr, param: id }, icon: git-pull-request }
",
        );
    write(
        root,
        ".oxplow/extensions/acme/merged.v1.json",
        r#"{"type": "object", "required": ["number"]}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.v2.json",
        r#"{"type": "object", "required": ["pr"]}"#,
    );
    write(
        root,
        ".oxplow/extensions/acme/merged.star",
        "def transform(x):\n    return {\"pr\": x[\"payload\"][\"number\"]}\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/datasets/prs.sql",
        "SELECT 'oxplow:acme/pr?id=1' AS uri, 'One' AS title",
    );
    write(
        root,
        ".oxplow/extensions/acme/pages/pr.yaml",
        "title: Pull request\nfor: acme/pr\ndefault: true\nparams: [{ name: uri }, { name: id }]\n\
             loaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/fixtures/merged.yaml",
        "when: { event_type: acme.merged, payload: { pr: 12 } }\nexpect: { valid: true }\n",
    );
    write(root, ".oxplow/extensions/acme/fixtures/old.yaml", "when: { event_type: acme.merged, v: 1, payload: { number: 12 } }\nexpect: { valid: true, upcast: { pr: 12 } }\n");
    write(
        root,
        ".oxplow/extensions/acme/fixtures/link.yaml",
        "when: { wikilink: \"pr:12\" }\nexpect: { uri: \"oxplow:acme/pr?id=12\" }\n",
    );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert!(report.ok, "{:?}", report.errors);
    for name in ["merged", "old", "link"] {
        assert!(
            report.ran.contains(&format!("example {name}")),
            "{:?}",
            report.ran
        );
    }

    write(
        root,
        ".oxplow/extensions/acme/fixtures/merged.yaml",
        "when: { event_type: acme.merged, payload: { number: 12 } }\nexpect: { valid: true }\n",
    );
    write(
        root,
        ".oxplow/extensions/acme/fixtures/link.yaml",
        "when: { wikilink: \"pr:12\" }\nexpect: { uri: \"oxplow:acme/pr?id=13\" }\n",
    );
    let errors = test_extension(root, "acme", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains(".oxplow/extensions/acme/fixtures/merged.yaml: `acme.merged@2` refuses"),
        "{errors}"
    );
    assert!(
        errors.contains(".oxplow/extensions/acme/fixtures/link.yaml: `[[pr:12]]` names"),
        "{errors}"
    );
}

/// A page's `viz: form` node is opened as a person would: its defaults
/// bound from the params, its fixed and shown fields; a wrong
/// expectation is a finding at its fixture.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_opens_a_form_page_and_compares_what_it_starts_with() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
            root,
            ".oxplow/extensions/editor/extension.yaml",
            "manifest: 2
name: editor
intent:
  purpose: Edit work items.
  examples:
    - { name: edit, input: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }, expect: the ref is fixed }
",
        );
    write(
            root,
            ".oxplow/extensions/editor/pages/edit-item.yaml",
            "title: Edit\nparams: [{ name: uri }]\nloaders:\n  item: { sql: \"SELECT title, body FROM v_work_item WHERE uri = :uri\" }\n\
             view: { viz: form, from: item, form: { command: oxplow.work_item.update, defaults: { uri: '{{param.uri}}' }, fixed: [uri], fields: [title, body] } }\n",
        );
    write(
            root,
            ".oxplow/extensions/editor/fixtures/edit.yaml",
            "requires: [oxplow-bundled]\nwhen: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }\nexpect: { values: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' }, fixed: [uri], fields: [title, body] }\n",
        );
    let report = test_extension(root, "editor", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.contains(&"example edit".to_string()),
        "{:?}",
        report.ran
    );

    write(
            root,
            ".oxplow/extensions/editor/fixtures/edit.yaml",
            "requires: [oxplow-bundled]\nwhen: { form: edit-item, params: { uri: 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' } }\nexpect: { fixed: [uri, title] }\n",
        );
    let errors = test_extension(root, "editor", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains(
            ".oxplow/extensions/editor/fixtures/edit.yaml:3: form `edit-item` opened with"
        ),
        "{errors}"
    );
}

/// A page with a work-item loader, one param and an answer.
fn pager(root: &Path, query: &str, answer_params: &str, expect: &str) {
    write(
        root,
        ".oxplow/extensions/pager/extension.yaml",
        "manifest: 2
name: pager
intent:
  purpose: Show work items.
  examples:
    - { name: created, input: { page: items }, expect: the created item is listed }
",
    );
    write(
        root,
        ".oxplow/extensions/pager/pages/items.yaml",
        &format!(
            "title: Items
params: [{{ name: state }}]
loaders:
  items: {{ sql: \"{query}\" }}
view:
  stack:
    - {{ id: list, title: Items, viz: table, from: items }}
answers:
  - {{ question: \"What is open?\", params: {answer_params} }}
"
        ),
    );
    write(
        root,
        ".oxplow/extensions/pager/fixtures/created.yaml",
        &format!(
            "requires: [oxplow-bundled]\ngiven:
  - {{ command: oxplow.work_item.create, input: {{ title: Write the page test }} }}
when: {{ page: items }}
expect: {expect}
"
        ),
    );
}

const LIST: &str = "SELECT title FROM v_work_item WHERE title = 'Write the page test'";

/// A part is tested with `when: { part }` as a page is, a query with
/// `when: { query }` against its last loader's rows (or a named loader's);
/// the wrong key is a finding that says which to write.
#[tokio::test(flavor = "multi_thread")]
async fn a_part_and_a_query_fixture_run_by_their_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/kinds/extension.yaml",
        "manifest: 2\nname: kinds\nintent:\n  purpose: test\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/parts/section.yaml",
        "title: Section\nloaders:\n  rows: { rows: [{ a: 1 }, { a: 2 }] }\nview: { id: list, viz: table, from: rows }\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/queries/options.yaml",
        "title: Options\nparams: [{ name: n, default: 2 }]\nloaders:\n  every: { rows: [{ value: 1, label: one }, { value: 2, label: two }] }\n  some: { sql: 'SELECT value, label FROM every WHERE value <= :n' }\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/fixtures/part.yaml",
        "when: { part: section }\nexpect: { loaders: { rows: { rows: 2 } }, nodes: [{ id: list, viz: table }] }\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/fixtures/query.yaml",
        "when: { query: options, params: { n: 1 } }\nexpect: { rows: [{ value: 1, label: one }], columns: [value, label] }\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/fixtures/query-loader.yaml",
        "when: { query: 'options#every' }\nexpect: { rows: 2 }\n",
    );
    let report = test_extension(root, "kinds", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(report.ok);

    write(
        root,
        ".oxplow/extensions/kinds/fixtures/wrong.yaml",
        "when: { page: section }\nexpect: { loaders: { rows: { rows: 2 } } }\n",
    );
    write(
        root,
        ".oxplow/extensions/kinds/fixtures/wrong-query.yaml",
        "when: { part: options }\nexpect: { loaders: { every: { rows: 2 } } }\n",
    );
    let errors = test_extension(root, "kinds", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("`page: section` names a part")
            && errors.contains("write `when: { part: section }`"),
        "{errors}"
    );
    assert!(
        errors.contains("`part: options` names a query")
            && errors.contains("write `when: { query: options }`"),
        "{errors}"
    );
}

/// A page fixture runs `given` steps first, then the page, and checks
/// its loaders and nodes; the answers run too.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_runs_a_page_after_its_given_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
            root,
            LIST,
            "{}",
            "{ loaders: { items: { rows: [{ title: Write the page test }], columns: [title] } }, nodes: [{ id: list, kind: viz }] }",
        );
    let report = test_extension(root, "pager", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report.ran.contains(&"example created".to_string()),
        "{:?}",
        report.ran
    );
    assert!(
        report.ran.contains(&"answers".to_string()),
        "{:?}",
        report.ran
    );
}

/// A node's `viz` is checked like its `kind`; a key `expect.nodes`
/// doesn't know is a finding, never ignored.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_checks_a_nodes_viz_and_refuses_keys_it_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
        root,
        LIST,
        "{}",
        "{ nodes: [{ id: list, kind: viz, viz: table }] }",
    );
    let report = test_extension(root, "pager", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    pager(root, LIST, "{}", "{ nodes: [{ id: list, viz: sections }] }");
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("node `list` is a `table` viz, the example expects `sections`"),
        "{errors}"
    );

    pager(root, LIST, "{}", "{ nodes: [{ id: list, title: Items }] }");
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("`title` isn't a key of `expect.nodes`")
            && errors.contains("`id`, `kind`, `viz`, `actions`"),
        "{errors}"
    );
}

/// A row that differs is a finding that names the loader, the place and
/// both values.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_names_the_page_row_that_differs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
        root,
        LIST,
        "{}",
        "{ loaders: { items: { rows: [{ title: Something else }] } } }",
    );
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/created.yaml:5: page `items` loader `items` returned")
            && errors.contains("Something else")
            && errors.contains("Write the page test"),
        "{errors}"
    );
}

/// An answer whose params break the page's query fails, naming the
/// page and the question.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_fails_an_answer_that_breaks_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
        root,
        // Runs at the page's defaults (no state); the answer's state
        // isn't JSON, so its run fails.
        "SELECT json_extract(:state, '$') AS state",
        "{ state: open }",
        "{ loaders: { items: { rows: $any } } }",
    );
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("pages/items.yaml: the answer to \"What is open?\" doesn't run"),
        "{errors}"
    );
}

/// `expect.text` checks a page's text: `contains` and the exact form.
#[tokio::test(flavor = "multi_thread")]
async fn extension_test_checks_a_pages_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
        root,
        LIST,
        "{}",
        "{ text: { contains: [Items, Write the page test] } }",
    );
    let report = test_extension(root, "pager", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    pager(root, LIST, "{}", "{ text: { contains: [Nowhere] } }");
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("text doesn't contain \"Nowhere\""),
        "{errors}"
    );

    pager(root, LIST, "{}", "{ text: nothing like it }");
    let errors = test_extension(root, "pager", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("the example expects \"nothing like it\""),
        "{errors}"
    );
}

/// Extension `paged` with page `items` (`page`, its YAML) and one
/// intent example per fixture, named by it.
fn paged(root: &Path, page: &str, fixtures: &[(&str, &str)]) {
    let examples: String = fixtures
        .iter()
        .map(|(name, _)| {
            format!("    - {{ name: {name}, input: {{ page: items }}, expect: it }}\n")
        })
        .collect();
    write(
            root,
            ".oxplow/extensions/paged/extension.yaml",
            &format!(
                "manifest: 2\nname: paged\nintent:\n  purpose: Show work items.\n  examples:\n{examples}"
            ),
        );
    write(root, ".oxplow/extensions/paged/pages/items.yaml", page);
    for (name, body) in fixtures {
        write(
            root,
            &format!(".oxplow/extensions/paged/fixtures/{name}.yaml"),
            body,
        );
    }
}

/// A page of one work item by `uri`, with a row action.
const BY_URI: &str = "title: Items
params: [{ name: uri }]
loaders:
  items: { sql: \"SELECT title, uri FROM v_work_item WHERE uri = :uri\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";

/// A fixture binds a step's result (`bind: <name>`) and a value of a
/// run (`steps[].bind`), and uses either as `$name.path` in a later
/// step's input, the params and the expectation; `steps` change the
/// state and run the page again.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_binds_values_and_runs_its_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
        root,
        BY_URI,
        &[(
            "bound",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First }, bind: first }
when: { page: items, params: { uri: $first.uri } }
expect:
  loaders: { items: { rows: [{ title: First, uri: $first.uri }] } }
  nodes: [{ id: items, kind: viz, actions: [start] }]
steps:
  - expect: { loaders: { items: { rows: 1 } } }
    bind: { title: 'loaders.items.rows[0].title' }
  - given: [{ command: oxplow.work_item.create, input: { title: Second }, bind: second }]
  - params: { uri: $second.uri }
    expect: { loaders: { items: { rows: [{ title: Second }] } } }
  - params: { uri: $first.uri }
    expect: { loaders: { items: { rows: [{ title: $title }] } } }
",
        )],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    // A step that doesn't hold names its place in the scenario.
    paged(
        root,
        BY_URI,
        &[(
            "bound",
            "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First }, bind: first }
when: { page: items, params: { uri: $first.uri } }
steps:
  - expect: { loaders: { items: { rows: [{ title: Other }] } } }
",
        )],
    );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/bound.yaml: steps[0]: page `items` loader `items` returned"),
        "{errors}"
    );

    // A name nothing bound is a finding that says how to bind it.
    paged(
        root,
        BY_URI,
        &[(
            "bound",
            "when: { page: items, params: { uri: $nowhere.uri } }\n",
        )],
    );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(errors.contains("`$nowhere` isn't bound"), "{errors}");
}

/// A loader's `contains` names rows it has, in any order, among
/// others; an action named by an object is compared on the fields it
/// names, spelled as the page YAML spells them.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_checks_rows_it_contains_and_an_actions_details() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let page = "title: Items
loaders:
  items: { sql: \"SELECT title, uri FROM v_work_item ORDER BY title\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";
    let given = "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: First } }
  - { command: oxplow.work_item.create, input: { title: Second } }
  - { command: oxplow.work_item.create, input: { title: Third } }
when: { page: items }
";
    paged(
        root,
        page,
        &[(
            "contains",
            &format!(
                "{given}expect:
  loaders: {{ items: {{ rows: {{ contains: [{{ title: Third }}, {{ title: First }}] }} }} }}
  nodes: [{{ id: items, actions: [{{ id: start, row: true, one_row: false }}] }}]
"
            ),
        )],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    paged(
        root,
        page,
        &[(
            "contains",
            &format!(
                "{given}expect:
  loaders: {{ items: {{ rows: {{ contains: [{{ title: Fourth }}] }} }} }}
  nodes: [{{ id: items, actions: [{{ id: start, row: false }}] }}]
"
            ),
        )],
    );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("loader `items` returned") && errors.contains("at `/rows/contains/0`"),
        "{errors}"
    );
    assert!(
        errors.contains("node `items`'s actions are true at `/0/row`"),
        "{errors}"
    );
}

/// A page step presses one of its actions for real, as a person would:
/// on a row (`row:` its columns), with what it answered or why it said
/// no checked by `expect`, and the page run again shows what it did.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_presses_an_action() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let page = "title: Items
params: [{ name: uri }]
loaders:
  items: { sql: \"SELECT title, uri, state FROM v_work_item WHERE uri = :uri\" }
view:
  from: items
  viz: table
  actions:
    - { id: start, label: Start, command: oxplow.work_item.transition, input: { uri: '{{row.uri}}', to: in_progress }, row: true }
";
    let fixture = |press: &str| {
        format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: First }}, bind: first }}]
when: {{ page: items, params: {{ uri: $first.uri }} }}
expect: {{ loaders: {{ items: {{ rows: [{{ state: todo }}] }} }} }}
steps:
  - {press}
  - expect: {{ loaders: {{ items: {{ rows: [{{ state: in_progress }}] }} }} }}
"
            )
    };
    paged(
        root,
        page,
        &[(
            "pressed",
            &fixture(
                "press: { action: start, node: items, row: { uri: $first.uri } }
    expect: { ok: true }",
            ),
        )],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    // A press that says no is checked like any outcome.
    paged(
            root,
            page,
            &[(
                "pressed",
                "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First }, bind: first }]
when: { page: items, params: { uri: $first.uri } }
steps:
  - press: { action: start, node: items, row: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\" } }
    expect: { ok: false, refused: $any }
",
            )],
        );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    // One that fails where the example expects it to work names its step.
    paged(
        root,
        page,
        &[(
            "pressed",
            &fixture("press: { action: begin, node: items, row: { uri: $first.uri } }"),
        )],
    );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/pressed.yaml: steps[0]: press `begin`"),
        "{errors}"
    );
}

/// A page step submits a form node with what a person would fill in,
/// and the command it names runs for real.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_submits_a_form() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
        root,
        "title: New item
loaders:
  items: { sql: \"SELECT title FROM v_work_item\" }
view:
  stack:
    - { id: add, title: Add, viz: form, form: { command: oxplow.work_item.create } }
    - { id: items, title: Items, viz: table, from: items }
",
        &[(
            "submitted",
            "requires: [oxplow-bundled]
when: { page: items }
expect: { loaders: { items: { rows: 0 } } }
steps:
  - submit: { node: add, input: { title: Written } }
    expect: { ok: true }
  - expect: { loaders: { items: { rows: [{ title: Written }] } } }
",
        )],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// What another extension mounts at a page's slot runs inside it: a
/// fixture of the mounting extension runs the host page and checks the
/// mount (`<mount id>`) and its loaders (`<mount id>/<loader>`).
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_checks_what_is_mounted_at_a_slot() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/host/extension.yaml",
        "manifest: 2\nname: host\nintent:\n  purpose: Hosts.\n",
    );
    write(
            root,
            ".oxplow/extensions/host/pages/items.yaml",
            "title: Items\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview:\n  stack:\n    - { id: count, title: Count, viz: number, from: n }\n    - { slot: extra }\n",
        );
    write(
            root,
            ".oxplow/extensions/beta/extension.yaml",
            "manifest: 2\nname: beta\nintent:\n  purpose: Mounts.\n  examples:\n    - { name: mounted, input: { page: host/items }, expect: it }\nrequires: [host]\nui:\n  mounts:\n    - { page: extra, at: host/items.extra }\n",
        );
    write(
            root,
            ".oxplow/extensions/beta/pages/extra.yaml",
            "title: Extra\nloaders:\n  words: { sql: \"SELECT 'hello' AS word\" }\nview: { viz: table, from: words }\n",
        );
    write(
            root,
            ".oxplow/extensions/beta/fixtures/mounted.yaml",
            "when: { page: host/items }\nexpect:\n  nodes: [{ id: beta.extra }]\n  loaders: { n: { rows: 1 }, beta.extra/words: { rows: [{ word: hello }] } }\n",
        );
    let report = test_extension(root, "beta", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// `when: { sql }` reads the published datasets as a page loader would —
/// a dataset, a decorator's labels, a metric — its rows checked like a
/// loader's.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_reads_the_datasets_with_sql() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2
name: acme
intent:
  purpose: PRs.
  examples:
    - { name: read, input: { sql: x }, expect: rows }
datasets:
  - name: prs
    version: 1
    description: PRs.
    fields:
      - { name: title, type: \"\", doc: Its title. }
",
    );
    write(
        root,
        ".oxplow/extensions/acme/datasets/prs.sql",
        "SELECT 'One' AS title UNION ALL SELECT 'Two'",
    );
    let fixture = |want: &str| {
        format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Three }} }}]
when: {{ sql: \"SELECT title FROM v_acme_prs UNION ALL SELECT title FROM v_work_item WHERE title = :t ORDER BY title\", params: {{ t: Three }} }}
expect: {{ rows: {want} }}
"
            )
    };
    write(
        root,
        ".oxplow/extensions/acme/fixtures/read.yaml",
        &fixture("[{ title: One }, { title: Three }, { title: Two }]"),
    );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    write(
        root,
        ".oxplow/extensions/acme/fixtures/read.yaml",
        &fixture("{ contains: [{ title: Four }] }"),
    );
    let errors = test_extension(root, "acme", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/read.yaml:4: page `query` loader `rows` returned"),
        "{errors}"
    );
}

/// `when: { command, input }` runs the command for real, as a person
/// (or `as: agent`): what it answered, or why it said no, and the
/// commands that ran because of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_runs_a_command_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
            root,
            BY_URI,
            &[
                (
                    "ran",
                    "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First }, bind: first }]
when: { command: oxplow.work_item.transition, input: { uri: $first.uri, to: in_progress } }
expect: { ok: true, commands: [] }
steps:
  - when: { input: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\", to: done } }
    expect: { ok: false, refused: $any }
",
                ),
                (
                    "wrong",
                    "requires: [oxplow-bundled]\nwhen: { command: oxplow.work_item.transition, input: { uri: \"oxplow:oxplow/work_item?id=tsk404&provider=oxplow\", to: done } }
expect: { ok: true }
",
                ),
            ],
        );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(!errors.contains("fixtures/ran.yaml"), "{errors}");
    assert!(
        errors.contains("fixtures/wrong.yaml:3: command `oxplow.work_item.transition` gave"),
        "{errors}"
    );
}

/// A materialized dataset is refilled before a fixture reads it, as the
/// app's change loop refills it: what a step made is in it.
#[tokio::test(flavor = "multi_thread")]
async fn a_materialized_dataset_is_current_when_a_fixture_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2
name: acme
intent:
  purpose: Titles.
  examples:
    - { name: stored, input: { sql: x }, expect: rows }
datasets:
  - name: titles
    version: 1
    key: [id]
    materialize: on_change
    description: Titles.
    fields:
      - { name: id, type: TEXT, doc: The item. }
      - { name: title, type: TEXT, doc: Its title. }
",
    );
    write(
        root,
        ".oxplow/extensions/acme/datasets/titles.sql",
        "SELECT uri AS id, title FROM ref('work_item')",
    );
    write(
            root,
            ".oxplow/extensions/acme/fixtures/stored.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Fix it } }]
when: { sql: \"SELECT title FROM v_acme_titles\" }
expect: { rows: [{ title: Fix it }] }
steps:
  - given: [{ command: oxplow.work_item.create, input: { title: Another } }]
  - expect: { rows: 2 }
",
        );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// `when: { advisory }` runs one of the extension's advisories for a
/// thread as the app would: its `lines` are what the agent is told.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_runs_an_advisory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2
name: acme
intent:
  purpose: Greets.
  examples:
    - { name: greeted, input: { advisory: hello }, expect: a greeting per item }
permissions: [agent.write]
advisories:
  - id: hello
    on: prompt
    once_per: turn
    heading: \"# Hello\"
    query: \"SELECT uri AS key, 'Hello ' || title AS message FROM v_work_item ORDER BY title\"
",
    );
    let fixture = |want: &str| {
        format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: Fix it }} }}]
when: {{ advisory: hello }}
expect: {{ lines: {want} }}
"
            )
    };
    write(
        root,
        ".oxplow/extensions/acme/fixtures/greeted.yaml",
        &fixture("{ contains: [Hello Fix it] }"),
    );
    let report = test_extension(root, "acme", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    write(
        root,
        ".oxplow/extensions/acme/fixtures/greeted.yaml",
        &fixture("[]"),
    );
    let errors = test_extension(root, "acme", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/greeted.yaml: advisory `hello` said"),
        "{errors}"
    );
}

/// A page's `ui:` step runs its custom component's own code in the
/// app's webview against the test's oxplow: what it draws, what it asks
/// the shell (a title, actions) and what it invokes, through clicks,
/// typing and a nav-bar press — and a wrong expectation names the step
/// and shows the frame.
#[tokio::test(flavor = "multi_thread")]
async fn a_ui_step_runs_the_component_in_the_apps_webview() {
    assert!(
        crate::component_harness::shell().is_some(),
        "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/cards/extension.yaml",
        "manifest: 2
name: cards
intent:
  purpose: Cards.
  examples:
    - { name: drawn, input: { page: board }, expect: cards }
permissions: [commands:oxplow.work_item.create]
custom_components:
  - { id: cards, title: Cards, commands: [oxplow.work_item.create] }
",
    );
    write(
            root,
            ".oxplow/extensions/cards/pages/board.yaml",
            "title: Board\nloaders:\n  items: { sql: \"SELECT title FROM v_work_item ORDER BY title\" }\nview: { component: { id: cards, props: { heading: Work } } }\n",
        );
    write(
            root,
            ".oxplow/extensions/cards/components/cards/index.html",
            "<!doctype html><html><head><title>cards</title></head><body>\
             <h1 id=\"heading\"></h1><ul id=\"list\"></ul><input id=\"t\"><button id=\"go\">Go</button>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
    write(
            root,
            ".oxplow/extensions/cards/components/cards/app.js",
            "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  var draw = function (data) {
    list.innerHTML = '';
    data.items.rows.forEach(function (r) {
      var li = document.createElement('li');
      li.textContent = r[0];
      list.appendChild(li);
    });
  };
  document.getElementById('heading').textContent = c.props.heading;
  draw(c.data);
  c.onUpdate(function (u) { draw(u.data); });
  c.setTitle('Cards: ' + c.data.items.rows.length);
  c.setActions([{ id: 'add', label: 'Add' }]);
  c.onAction(function (id) { if (id === 'add') c.invoke('oxplow.work_item.create', { title: 'From the bar' }); });
  document.getElementById('go').addEventListener('click', function () {
    c.invoke('oxplow.work_item.create', { title: document.getElementById('t').value });
  });
});
",
        );
    let fixture = |last: u32| {
        format!(
                "requires: [oxplow-bundled]\ngiven: [{{ command: oxplow.work_item.create, input: {{ title: First }} }}]
when: {{ page: board }}
steps:
  - ui:
      - expect: {{ count: {{ li: 1 }}, text: {{ \"#heading\": Work }}, title: \"Cards: 1\", actions: [add] }}
      - fill: {{ \"#t\": Second }}
      - click: \"#go\"
      - expect: {{ count: {{ li: 2 }}, invoked: [{{ command: oxplow.work_item.create, ok: true }}] }}
      - press: add
      - expect: {{ count: {{ li: {last} }} }}
"
            )
    };
    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        &fixture(3),
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
    if !report.skipped.is_empty() {
        // The display is locked or asleep: the window draws nothing, so the
        // steps were not run (`a_window_that_draws_nothing_…` covers that).
        return;
    }

    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        &fixture(5),
    );
    let errors = test_extension(root, "cards", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("fixtures/drawn.yaml: steps[0]: ui[5]:")
            && errors.contains("<li>From the bar</li>"),
        "{errors}"
    );
}

/// A window that draws no frames (the display locked or asleep) leaves
/// a fixture's `ui` steps, and the steps after them, not run, with why,
/// never failed. The shell is
/// a stand-in that says so, as the harness's page does.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_draws_nothing_leaves_the_ui_steps_unrun() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let shell = root.join("fake-shell");
    std::fs::write(
        &shell,
        r#"#!/bin/sh
base="${2%/harness/}"
post() { curl -s -X POST -H 'content-type: application/json' -d "$2" "$base$1"; }
post /harness/record '{"kind":"drawing","value":false}' >/dev/null
while :; do case "$(post /harness/next '{"result":null}')" in *done*) exit 0;; esac; done
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("OXPLOW_SHELL", &shell);
    component_ext(root, "oxplow.connect();");
    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        "when: { page: board }\nsteps:\n  - ui:\n      - expect: { count: { li: 99 } }\n  \
             - when: { sql: SELECT 1 AS n }\n    expect: { rows: [{ n: 2 }] }\n",
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    // What follows the `ui` steps reads what they did: it isn't run either.
    assert_eq!(report.error_lines(), Vec::<String>::new());
    assert!(
        report
            .skipped_lines()
            .iter()
            .any(|w| w.contains("the steps after them, aren't run") && w.contains("display")),
        "{:?}",
        report.skipped
    );
    // The case is skipped, never passed: the Tests panel shows it unrun.
    let case = report
        .cases
        .iter()
        .find(|c| c.name == "example drawn")
        .unwrap();
    assert_eq!(case.outcome, CaseOutcome::Skipped, "{case:?}");
    let xml = render(&report, Format::Junit);
    assert!(xml.contains("skipped=\"1\""), "{xml}");
    assert!(xml.contains("<testcase name=\"example drawn\""), "{xml}");
    assert!(xml.contains("<skipped message=\""), "{xml}");
    assert!(render(&report, Format::Text).contains("1 skipped"));
    // The same skip in several fixtures is said once, naming each file.
    write(
        root,
        ".oxplow/extensions/cards/fixtures/also.yaml",
        "when: { page: board }\nsteps:\n  - ui:\n      - expect: { count: { li: 99 } }\n",
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    assert_eq!(report.skipped.len(), 2, "{:?}", report.skipped);
    let text = render(&report, Format::Text);
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("skipped:")).collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(
        lines[0].contains("fixtures/also.yaml") && lines[0].contains("fixtures/drawn.yaml"),
        "{text}"
    );
}

/// With `--require-ui`, a window that draws nothing fails the case: the
/// run wanted the `ui` steps run, so not running them is an error.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_draws_nothing_fails_the_ui_steps_when_ui_is_required() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let shell = root.join("fake-shell");
    std::fs::write(
        &shell,
        r#"#!/bin/sh
base="${2%/harness/}"
post() { curl -s -X POST -H 'content-type: application/json' -d "$2" "$base$1"; }
post /harness/record '{"kind":"drawing","value":false}' >/dev/null
while :; do case "$(post /harness/next '{"result":null}')" in *done*) exit 0;; esac; done
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("OXPLOW_SHELL", &shell);
    component_ext(root, "oxplow.connect();");
    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        "when: { page: board }\nsteps:\n  - ui:\n      - expect: { count: { li: 99 } }\n",
    );
    let report = test_extension_with(root, "cards", false, true)
        .await
        .unwrap();
    assert!(!report.ok, "{report:?}");
    assert!(
        report
            .error_lines()
            .iter()
            .any(|e| e.contains("the steps after them, aren't run")
                && e.contains("display")
                && e.contains("--require-ui")),
        "{:?}",
        report.errors
    );
    assert_eq!(report.skipped_lines(), Vec::<String>::new());
    let case = report
        .cases
        .iter()
        .find(|c| c.name == "example drawn")
        .unwrap();
    assert_eq!(case.outcome, CaseOutcome::Failed, "{case:?}");
}

/// A component page of one custom component, for the `ui` tests: its
/// `app.js` runs on `oxplow.connect()`'s component.
fn component_ext(root: &Path, app: &str) {
    write(
        root,
        ".oxplow/extensions/cards/extension.yaml",
        "manifest: 2
name: cards
intent:
  purpose: Cards.
  examples:
    - { name: drawn, input: { page: board }, expect: cards }
permissions: [commands:oxplow.work_item.create]
custom_components:
  - { id: cards, title: Cards, commands: [oxplow.work_item.create] }
",
    );
    write(
            root,
            ".oxplow/extensions/cards/pages/board.yaml",
            "title: Board
params:
  - { name: kind, role: filter, default: all }
loaders:
  items: { sql: \"SELECT title FROM v_work_item WHERE :kind = 'all' OR title LIKE :kind || '%' ORDER BY title\" }
view: { component: { id: cards } }
",
        );
    write(
            root,
            ".oxplow/extensions/cards/components/cards/index.html",
            "<!doctype html><html><body><form id=\"f\"><input id=\"t\"><span id=\"err\"></span></form><ul id=\"list\"></ul>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
    write(
        root,
        ".oxplow/extensions/cards/components/cards/app.js",
        app,
    );
}

/// Enter in a field of a form submits it, as a browser does: the form's
/// own `submit` handler runs, and one that stops the keydown keeps it
/// from submitting.
#[tokio::test(flavor = "multi_thread")]
async fn a_ui_enter_submits_the_form_a_field_is_in() {
    assert!(
        crate::component_harness::shell().is_some(),
        "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    component_ext(
        root,
        "oxplow.connect().then(function (c) {
  var f = document.getElementById('f'), t = document.getElementById('t');
  f.addEventListener('submit', function (e) {
    e.preventDefault();
    if (!t.value) { document.getElementById('err').textContent = 'Required'; return; }
    document.getElementById('err').textContent = '';
    c.invoke('oxplow.work_item.create', { title: t.value });
  });
});
",
    );
    write(
            root,
            ".oxplow/extensions/cards/fixtures/drawn.yaml",
            "requires: [oxplow-bundled]\nwhen: { page: board }
steps:
  - ui:
      - key: { on: \"#t\", key: Enter }
      - expect: { text: { \"#err\": Required }, invoked: [] }
      - fill: { \"#t\": Second }
      - key: { on: \"#t\", key: Enter }
      - expect: { text: { \"#err\": \"\" }, invoked: [{ command: oxplow.work_item.create, ok: true }] }
",
        );
    let report = test_extension(root, "cards", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// A component's view params re-run its page, as the app's tab does: a
/// filter it sets is the page's param for the next run, and Back
/// (`view_params` with `null`) drops it.
#[tokio::test(flavor = "multi_thread")]
async fn a_ui_view_param_runs_the_page_again_with_it() {
    assert!(
        crate::component_harness::shell().is_some(),
        "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    component_ext(
        root,
        "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  var draw = function (data) {
    list.innerHTML = '';
    data.items.rows.forEach(function (r) {
      var li = document.createElement('li');
      li.textContent = r[0];
      list.appendChild(li);
    });
  };
  draw(c.data);
  c.onUpdate(function (u) { draw(u.data); });
  var b = document.createElement('button');
  b.id = 'only-b';
  b.textContent = 'B';
  b.addEventListener('click', function () { c.setViewParams({ kind: 'B' }); });
  document.body.appendChild(b);
});
",
    );
    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: Alpha } }
  - { command: oxplow.work_item.create, input: { title: Beta } }
when: { page: board }
steps:
  - ui:
      - expect: { count: { li: 2 } }
      - click: \"#only-b\"
      - expect: { count: { li: 1 }, text: { li: Beta }, view_params: { kind: B } }
      - view_params: { kind: null }
      - expect: { count: { li: 2 } }
",
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// `shown` is the view the core drew: the spec when it opened, `null`
/// when the core refused it or the component cleared it — however fast
/// the component asks.
#[tokio::test(flavor = "multi_thread")]
async fn a_ui_shown_is_the_last_view_the_core_drew() {
    assert!(
        crate::component_harness::shell().is_some(),
        "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    component_ext(
            root,
            "oxplow.connect().then(function (c) {
  var spec = function (sql) {
    return 'title: Out\\nloaders:\\n  rows: { sql: \"' + sql + '\" }\\nview: { from: rows, viz: table }\\n';
  };
  var button = function (id, run) {
    var b = document.createElement('button');
    b.id = id;
    b.addEventListener('click', run);
    document.body.appendChild(b);
  };
  button('bad', function () { c.showView(spec('SELECT * FROM no_such_model')).catch(function () {}); });
  button('good', function () { c.showView(spec('SELECT 1 AS n')).catch(function () {}); });
  button('clear', function () { c.showView(null); });
  // Three asks at once: the last the core drew is the one that counts.
  button('burst', function () {
    c.showView(spec('SELECT 1 AS n')).catch(function () {});
    c.showView(spec('SELECT * FROM no_such_model')).catch(function () {});
    c.showView(spec('SELECT 2 AS n')).catch(function () {});
  });
});
",
        );
    write(
        root,
        ".oxplow/extensions/cards/fixtures/drawn.yaml",
        r##"when: { page: board }
steps:
  - ui:
      - click: "#bad"
      - expect: { shown: null }
      - click: "#good"
      - expect: { shown: "title: Out\nloaders:\n  rows: { sql: \"SELECT 1 AS n\" }\nview: { from: rows, viz: table }\n" }
      - click: "#bad"
      - expect: { shown: null }
      - click: "#good"
      - click: "#clear"
      - expect: { shown: null }
      - click: "#burst"
      - expect: { shown: "title: Out\nloaders:\n  rows: { sql: \"SELECT 2 AS n\" }\nview: { from: rows, viz: table }\n" }
"##,
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// A `ui` step drives a component inside what the page embeds — here
/// the page another extension stands in with for a replaceable node —
/// the first one found, or the one `node: <embed>/<node>` names.
#[tokio::test(flavor = "multi_thread")]
async fn a_ui_step_drives_a_component_a_replacement_stands_in_with() {
    assert!(
        crate::component_harness::shell().is_some(),
        "no oxplow app to draw the harness: build it first (`bun run build:dev`)"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/host/extension.yaml",
        "manifest: 2\nname: host\nintent:\n  purpose: Hosts.\n",
    );
    write(
            root,
            ".oxplow/extensions/host/pages/board.yaml",
            "title: Board\nparams: [{ name: scope, default: all }, { name: thread_id }]\nloaders:\n  own: { sql: \"SELECT 1 AS n\" }\nview: { id: cards, viz: table, from: own, replaceable: work_item.board }\n",
        );
    write(
        root,
        ".oxplow/extensions/lanes/extension.yaml",
        "manifest: 2
name: lanes
intent:
  purpose: Lanes.
  examples:
    - { name: drawn, input: { page: host/board }, expect: lanes }
requires: [host]
custom_components:
  - { id: lanes, title: Lanes }
ui:
  mounts:
    - { page: board, at: work_item.board }
",
    );
    write(
            root,
            ".oxplow/extensions/lanes/pages/board.yaml",
            "title: Lanes\nparams: [{ name: scope }, { name: thread_id }]\nloaders:\n  items: { sql: \"SELECT title FROM v_work_item ORDER BY title\" }\nview: { id: board, component: { id: lanes } }\n",
        );
    write(
            root,
            ".oxplow/extensions/lanes/components/lanes/index.html",
            "<!doctype html><html><head><title>lanes</title></head><body><ul id=\"list\"></ul>\
             <script src=\"/component-lib/oxplow-component.js\"></script><script src=\"app.js\"></script></body></html>",
        );
    write(
        root,
        ".oxplow/extensions/lanes/components/lanes/app.js",
        "oxplow.connect().then(function (c) {
  var list = document.getElementById('list');
  c.data.items.rows.forEach(function (r) {
    var li = document.createElement('li');
    li.textContent = r[0];
    list.appendChild(li);
  });
});
",
    );
    write(
            root,
            ".oxplow/extensions/lanes/fixtures/drawn.yaml",
            "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: First } }]
when: { page: host/board }
expect: { nodes: [{ id: cards/board }] }
steps:
  - ui:
      - expect: { text: { li: First } }
  - node: cards/board
    ui:
      - expect: { count: { li: 1 } }
",
        );
    let report = test_extension(root, "lanes", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// `rows: []` is no rows, like `rows: 0`.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_row_list_is_no_rows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
            root,
            "title: Items\nloaders:\n  items: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: items }\n",
            &[("empty", "when: { page: items }\nexpect: { loaders: { items: { rows: [] } } }\n")],
        );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(errors.contains("loader `items` returned"), "{errors}");
}

/// Each example with `given` runs on an oxplow of its own: what one
/// made, the next doesn't see.
#[tokio::test(flavor = "multi_thread")]
async fn page_fixtures_dont_see_each_others_data() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let fixture = "requires: [oxplow-bundled]\ngiven: [{ command: oxplow.work_item.create, input: { title: Mine } }]
when: { page: items }
expect: { loaders: { items: { rows: 1 } } }
";
    paged(
        root,
        "title: Items
loaders:
  items: { sql: \"SELECT title FROM v_work_item WHERE title = 'Mine'\" }
view: { from: items, viz: table }
",
        &[("one", fixture), ("two", fixture)],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// `confirmed: true` runs a step that asks for confirmation; `view:`
/// gives the run its stream and thread; `expect.alert` checks the alert.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_fixture_confirms_views_from_a_thread_and_checks_the_alert() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let page = "title: Items
params: [{ name: thread_id }]
loaders:
  items: { sql: \"SELECT :thread_id AS thread\" }
view: { from: items, viz: table }
alert: { min_rows: 1, label: Configured }
";
    paged(
        root,
        page,
        &[(
            "viewed",
            "view: { stream: str1, thread: thr1 }
given:
  - { command: oxplow.ai.set_provider, input: { id: spare, kind: anthropic, key: sk-ant-test } }
  - { command: oxplow.ai.remove_provider, input: { id: spare }, confirmed: true }
when: { page: items }
expect:
  loaders: { items: { rows: [{ thread: 1 }] } }
  alert: { firing: true, count: 1, message: $any }
",
        )],
    );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());

    paged(
        root,
        page,
        &[(
            "viewed",
            "given:
  - { command: oxplow.ai.set_provider, input: { id: spare, kind: anthropic, key: sk-ant-test } }
  - { command: oxplow.ai.remove_provider, input: { id: spare } }
when: { page: items }
",
        )],
    );
    let errors = test_extension(root, "paged", false)
        .await
        .unwrap()
        .error_lines()
        .join("\n");
    assert!(
        errors.contains("`given[1]` `oxplow.ai.remove_provider` failed")
            && errors.contains("confirmed: true"),
        "{errors}"
    );
}

/// The JUnit form has a case per thing that ran, a failure holding the
/// findings it added.
#[tokio::test(flavor = "multi_thread")]
async fn a_report_renders_as_junit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    pager(
        root,
        LIST,
        "{}",
        "{ loaders: { items: { rows: [{ title: Something else }] } } }",
    );
    let report = test_extension(root, "pager", false).await.unwrap();
    let xml = render(&report, Format::Junit);
    assert!(
        xml.starts_with("<testsuite name=\"pager\" tests=\""),
        "{xml}"
    );
    assert!(
        xml.contains("<testcase name=\"check\" classname=\"pager\""),
        "{xml}"
    );
    assert!(
        xml.contains("<testcase name=\"example created\" classname=\"pager\"")
            && xml.contains("<failure message=\"")
            && xml.contains("Something else"),
        "{xml}"
    );
    assert!(xml.contains("<testcase name=\"answers items\""), "{xml}");
}

/// What a step's command sets off — a reactor hearing its event —
/// has happened before the next step and the run: switching the work
/// list off through its command empties the work-item interface.
#[tokio::test(flavor = "multi_thread")]
async fn a_steps_reactions_land_before_the_page_runs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    paged(
            root,
            "title: Items
loaders:
  items: { sql: \"SELECT title FROM v_work_item\" }
view: { from: items, viz: table }
",
            &[(
                "switched",
                "requires: [oxplow-bundled]\ngiven:
  - { command: oxplow.work_item.create, input: { title: Filed } }
when: { page: items }
expect: { loaders: { items: { rows: [{ title: Filed }] } } }
steps:
  - given: [{ command: oxplow.capability.choose, input: { capability: work_items, provider: none }, confirmed: true }]
  - expect: { loaders: { items: { rows: 0 } } }
",
            )],
        );
    let report = test_extension(root, "paged", false).await.unwrap();
    assert_eq!(report.error_lines(), Vec::<String>::new());
}

/// A problem in an extension under test names the file to edit — its
/// source folder — once, never the copy the throwaway oxplow runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_problem_names_the_source_file_once_not_the_running_copy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        ".oxplow/extensions/cards/extension.yaml",
        "manifest: 2
name: cards
intent:
  purpose: Cards.
permissions: [commands:cards.nothing]
custom_components:
  - { id: cards, title: Cards, commands: [cards.nothing] }
",
    );
    write(
        root,
        ".oxplow/extensions/cards/components/cards/index.html",
        "<!doctype html><html><head><title>cards</title></head><body></body></html>",
    );
    let report = test_extension(root, "cards", false).await.unwrap();
    let named: Vec<_> = report
        .errors
        .iter()
        .filter(|e| e.message.contains("`cards.nothing`"))
        .collect();
    assert_eq!(named.len(), 1, "{:#?}", report.errors);
    assert_eq!(named[0].file.as_deref(), Some("extension.yaml"));
    let shown = named[0].render(&report.root);
    assert!(
        shown.starts_with(".oxplow/extensions/cards/extension.yaml"),
        "{shown}"
    );
    let rendered = report.error_lines();
    assert!(
        rendered.iter().all(|e| !e.contains("runtime/extensions")
            && !e.contains(root.to_str().unwrap())
            && !e.starts_with('/')),
        "{rendered:#?}"
    );
}

/// An extension kept under `extensions/<name>` (`extensionSources`): a page's
/// loader that fails its dry run, a fixture whose comparison doesn't hold and
/// an intent example no fixture names are each reported in that source
/// folder, never under the copy the throwaway runs.
fn kept_under_extensions(root: &std::path::Path, loader_sql: &str) {
    write(
        root,
        ".oxplow/project.yaml",
        "extensionSources:\n  lists: extensions/lists\n",
    );
    write(
        root,
        "extensions/lists/extension.yaml",
        "manifest: 2
name: lists
intent:
  purpose: Lists.
  examples:
    - name: shows-titles
      input: { page: ok }
      expect: a number
    - name: has-no-fixture
      input: { page: ok }
      expect: nothing
",
    );
    write(
        root,
        "extensions/lists/pages/titles.yaml",
        &format!(
            "title: Titles
loaders:
  titles: {{ sql: \"{loader_sql}\" }}
view: {{ from: titles, viz: table }}
"
        ),
    );
    write(
        root,
        "extensions/lists/pages/ok.yaml",
        "title: Ok
loaders:
  ok: { rows: [{ n: 1 }] }
view: { from: ok, viz: table }
",
    );
    write(
        root,
        "extensions/lists/fixtures/shows-titles.yaml",
        "when: { page: ok }
expect: { loaders: { ok: { rows: [{ n: 2 }] } } }
",
    );
}

fn in_the_source_folder(root: &std::path::Path, report: &TestReport) -> Vec<String> {
    let lines: Vec<String> = report
        .errors
        .iter()
        .chain(report.warnings.iter())
        .map(|d| d.render(&report.root))
        .collect();
    assert!(
        lines
            .iter()
            .all(|l| !l.contains("runtime/extensions") && !l.contains(root.to_str().unwrap())),
        "{lines:#?}"
    );
    lines
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loader_that_fails_names_the_source_folder_not_the_running_copy() {
    let dir = tempfile::tempdir().unwrap();
    kept_under_extensions(dir.path(), "SELECT nope FROM v_work_item");
    let report = test_extension(dir.path(), "lists", false).await.unwrap();
    let lines = in_the_source_folder(dir.path(), &report);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("extensions/lists/pages/titles.yaml") && l.contains("nope")),
        "{lines:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fixture_that_differs_and_an_example_with_no_fixture_name_the_source_folder() {
    let dir = tempfile::tempdir().unwrap();
    kept_under_extensions(dir.path(), "SELECT title FROM v_work_item");
    let report = test_extension(dir.path(), "lists", false).await.unwrap();
    let lines = in_the_source_folder(dir.path(), &report);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("extensions/lists/fixtures/shows-titles.yaml")),
        "the differing fixture: {lines:#?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("has-no-fixture")),
        "the example with no fixture: {lines:#?}"
    );
}

/// An extension that ships with oxplow runs the copy built into it, so a
/// problem is found in that copy; the report still names the folder a
/// person edits — its source in the checkout, `extensions/<name>` — and
/// never the deployed copy (a path under the throwaway's runtime store).
#[tokio::test(flavor = "multi_thread")]
async fn a_shipped_extension_is_reported_under_its_source_folder() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".oxplow/project.yaml",
        "extensionSources:\n  oxplow-bundled: extensions/oxplow-bundled\n",
    );
    write(
        dir.path(),
        "extensions/oxplow-bundled/extension.yaml",
        "manifest: 2\nname: oxplow-bundled\nintent:\n  purpose: x\n",
    );
    let report = test_extension_only(dir.path(), "oxplow-bundled", &["none".into()], false)
        .await
        .unwrap();
    assert_eq!(
        report.root, "extensions/oxplow-bundled",
        "{:#?}",
        report.errors
    );
    let shown: Vec<String> = report
        .errors
        .iter()
        .map(|d| d.render(&report.root))
        .collect();
    assert!(
        shown.iter().all(|l| !l.contains("runtime/extensions")),
        "{shown:#?}"
    );
}
