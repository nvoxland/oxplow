//! A project with extension `acme` holding the pages the page tests run.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::runner::*;
use crate::pages::spec::ViewContext;

pub(crate) fn write(root: &Path, rel: &str, text: &str) {
    let file = root.join(rel);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

pub(crate) const PAGES: &[(&str, &str)] = &[
    (
        "numbers",
        r#"
title: Numbers
keywords: digits counting
params: [{ name: n, default: 2 }]
answers:
  - { question: "Which numbers are big?", params: { n: 2 } }
loaders:
  base: { sql: "SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3" }
  big: { sql: "SELECT n FROM base WHERE n >= :n ORDER BY n" }
  count: { sql: "SELECT count(*) AS total FROM big" }
view:
  stack:
    - { title: Big ones, viz: table, from: big }
    - { title: Total, hide_title: true, viz: number, from: count }
"#,
    ),
    (
        "picked",
        r#"
title: Picked
params: [{ name: total }]
loaders:
  echo: { sql: "SELECT :total AS got" }
view: { viz: number, from: echo }
"#,
    ),
    (
        "host",
        r#"
title: Host
loaders:
  first: { sql: "SELECT 3 AS total" }
  borrowed: { page: acme/numbers, loader: count, params: { n: 1 } }
  colors: { rows: [{ state: failed }, { state: passed }] }
view:
  stack:
    - { embed: { page: acme/picked } }
    - { title: Borrowed, viz: number, from: borrowed }
    - { title: Colors, viz: list, from: colors }
"#,
    ),
    (
        "chooser",
        r#"
title: Chooser
params:
  - { name: start, default: 'snap:1' }
  - { name: end, type: choice, options_from: ends }
  - { name: mode, type: choice, options_from: '#modes' }
loaders:
  modes: { rows: [{ value: a, label: A }, { value: b, label: B }] }
  shown: { sql: "SELECT :start AS start, :end AS end_, :mode AS mode" }
view: { viz: table, from: shown }
"#,
    ),
    (
        "badchooser",
        r#"
title: Bad chooser
params:
  - { name: pick, type: choice, options_from: plain }
loaders:
  shown: { sql: "SELECT :pick AS pick" }
view: { viz: table, from: shown }
"#,
    ),
    (
        "sparse",
        r#"
title: Sparse
loaders:
  some: { sql: "SELECT 'x' AS a" }
  none: { sql: "SELECT 'x' AS a WHERE 0" }
view:
  stack:
    - { id: some, title: Some, viz: list, from: some, hide_empty: true }
    - { id: none, title: None, viz: list, from: none, hide_empty: true }
    - { id: shown, title: Shown, viz: list, from: none, empty: Nothing here. }
"#,
    ),
    (
        "areas",
        r#"
title: Areas
params: [{ name: area, role: filter, default: one }]
loaders:
  one: { sql: "SELECT 'one' AS a" }
  two: { sql: "SELECT 'two' AS a" }
view:
  switch:
    param: area
    children:
      - { id: one, title: One, viz: list, from: one }
      - { id: two, title: Two, viz: list, from: two }
"#,
    ),
    (
        "queried",
        r#"
title: Queried
loaders:
  q:
    command: oxplow.sql.query
    args: { sql: "SELECT 7 AS seven" }
    reads: [v_dataset]
view: { viz: table, from: q }
"#,
    ),
    (
        "listed",
        r#"
title: Listed
loaders:
  nodes:
    command: oxplow.page.describe
    args: { uri: "oxplow:oxplow/file?filesystem=oxplow:extensions&path=acme/pages/numbers.yaml" }
    items: nodes
    reads: [v_page]
  missing:
    command: oxplow.page.describe
    args: { uri: "oxplow:oxplow/file?filesystem=oxplow:extensions&path=acme/pages/numbers.yaml" }
    items: nope
    reads: [v_page]
view: { viz: table, from: nodes }
"#,
    ),
    (
        "emptied",
        r#"
title: Emptied
loaders:
  none: { rows: [] }
  after_none: { sql: "SELECT state FROM none" }
  broken:
    command: oxplow.page.describe
    args: { uri: "oxplow:oxplow/file?filesystem=oxplow:extensions&path=acme/pages/numbers.yaml" }
    items: nope
    reads: [v_page]
  after_broken: { sql: "SELECT id FROM broken" }
view: { viz: table, from: after_none }
"#,
    ),
    (
        "bound",
        r#"
title: Bound
loaders:
  row: { sql: "SELECT 42 AS answer" }
  echo:
    sql: "SELECT :answer AS got"
    params: { answer: { from: row.answer } }
view: { viz: number, from: echo }
"#,
    ),
    (
        "acts",
        r#"
title: Acts
params: [{ name: t, default: from-page }]
loaders:
  names: { sql: "SELECT 'alpha' AS name UNION ALL SELECT 'beta'" }
actions:
  - { id: add, label: Add, command: oxplow.work_item.create, input: { title: "{{param.t}}" } }
view:
  viz: list
  from: names
  actions:
    - { id: add-row, label: Add this, command: oxplow.work_item.create, input: { title: "{{row.name}}" }, row: true }
"#,
    ),
    (
        "commit-look",
        r#"
title: Commit Look
description: A commit, looked at.
for: oxplow/commit
params: [{ name: uri }, { name: sha }]
answers:
  - { question: "What did this commit change?", about: oxplow/commit }
loaders:
  a: { sql: "SELECT :sha AS s" }
view: { viz: number, from: a }
"#,
    ),
    (
        "file-look",
        r#"
title: File Look
for: oxplow/file
params: [{ name: uri }]
content: { text: true, max_bytes: 8 }
loaders:
  shown: { sql: "SELECT path, text, truncated FROM content" }
  bare: { sql: "SELECT count(*) AS n FROM content" }
view: { viz: table, from: shown }
"#,
    ),
    (
        "needy",
        r#"
title: Needy
needs: [work_items]
loaders:
  q: { sql: "SELECT 1 AS n" }
view: { viz: table, from: q }
"#,
    ),
    (
        "headed",
        r#"
title: Headed
params: [{ name: head_sha }, { name: stream_id }]
loaders:
  at: { sql: "SELECT :head_sha AS head" }
  other: { sql: "SELECT 1 AS n" }
view: { viz: table, from: at }
"#,
    ),
    (
        "rename",
        r#"
title: Rename
loaders:
  row: { sql: "SELECT 'sess-1' AS session" }
actions:
  - { id: rename, label: Rename…, command: oxplow.work_item.create, input: { title: "{{row.session}}" }, form: true }
view: { viz: detail, from: row }
"#,
    ),
    (
        "hosts-rename",
        r#"
title: Hosts Rename
view:
  stack:
    - id: inner
      embed: { page: acme/rename }
"#,
    ),
    (
        "session",
        r#"
title: Session
loaders:
  row: { sql: "SELECT 'sess-1' AS session, 1 AS running" }
  other: { sql: "SELECT 1 AS n UNION ALL SELECT 2" }
actions:
  - { id: stop, label: Stop, command: oxplow.work_item.create, input: { title: "stop {{row.session}}" }, when: running }
view: { viz: table, from: other }
"#,
    ),
    (
        "opens",
        r#"
title: Opens
loaders:
  row: { sql: "SELECT 'oxplow:oxplow/file?path=a.rs' AS uri, '' AS none" }
actions:
  - { id: show, label: Show, open: uri, when: uri }
  - { id: nothing, label: Nothing, open: none }
view: { viz: detail, from: row }
"#,
    ),
    (
        "new-item",
        r#"
title: New Item
params: [{ name: t, default: drafted }]
loaders:
  states:
    rows: [{ value: todo, label: To do }, { value: done, label: Done }]
view:
  viz: form
  form:
    command: oxplow.work_item.create
    defaults: { title: "{{param.t}}" }
    choices: { state: acme/new-item#states }
"#,
    ),
    (
        "slotted",
        r#"
title: Slotted
params: [{ name: n, default: 4 }]
loaders:
  first: { sql: "SELECT 1 AS a" }
  last: { sql: "SELECT 2 AS b" }
view:
  stack:
    - { title: First, viz: number, from: first }
    - { slot: sections }
    - { title: Last, viz: number, from: last }
"#,
    ),
    (
        "one_measure",
        r#"
title: One measure
loaders:
  facts: { sql: "SELECT count(*) AS n, abs(random()) AS r FROM v_fact WHERE measure_key = read_measure('acme.size')" }
view: { viz: table, from: facts }
"#,
    ),
    (
        "mixed",
        r#"
title: Mixed
loaders:
  steady: { sql: "SELECT abs(random()) AS r" }
  datasets: { sql: "SELECT count(*) AS n FROM v_dataset" }
  after: { sql: "SELECT n, abs(random()) AS r FROM datasets" }
view:
  stack:
    - { title: Steady, viz: number, from: steady }
    - { title: After, viz: table, from: after }
"#,
    ),
    (
        "related",
        r#"
title: Related
description: A page that links to others.
params:
  - { name: n, default: 3 }
  - { name: pick, role: filter }
loaders:
  links:
    rows:
      - { title: Docs, uri: "oxplow:oxplow/page?page=acme/numbers", description: The numbers. }
      - { title: Docs again, uri: "oxplow:oxplow/page?page=acme/numbers" }
      - { title: Empty uri, uri: "" }
  bare: { rows: [{ name: x }] }
view: { viz: table, from: links }
see_also:
  - { page: numbers, params: { n: { param: n } } }
  - { page: beta/nowhere }
  - { from: links }
  - { from: bare }
"#,
    ),
    (
        "led",
        r#"
title: Led
loaders:
  about: { rows: [{ description: This is what it is., fact: 1 }] }
  blank: { rows: [{ description: "", fact: 2 }] }
view:
  stack:
    - { id: with, title: With, viz: detail, from: about, lead: description, columns: [{ key: fact, label: Fact }] }
    - { id: without, title: Without, viz: detail, from: blank, lead: description, columns: [{ key: fact, label: Fact }] }
"#,
    ),
    (
        "writes",
        r#"
title: Writes
loaders:
  w:
    command: oxplow.work_item.create
    args: { title: nope }
    refresh: manual
view: { viz: table, from: w }
"#,
    ),
];

/// The queries beside `PAGES`: what a choice's `options_from` names.
pub(crate) const QUERIES: &[(&str, &str)] = &[
    (
        "ends",
        r#"
title: Ends
params: [{ name: start }]
loaders:
  points: { rows: [{ value: 'snap:1', label: One }, { value: 'snap:2', label: Two }] }
  after: { sql: "SELECT value, label FROM points WHERE value > :start" }
"#,
    ),
    (
        "plain",
        r#"
title: Plain
loaders:
  rows: { sql: "SELECT 1 AS v" }
"#,
    ),
];

pub(crate) async fn fixture() -> (tempfile::TempDir, Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    write(
        dir.path(),
        ".oxplow/extensions/acme/extension.yaml",
        "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
    );
    for (slug, text) in PAGES {
        write(
            dir.path(),
            &format!(".oxplow/extensions/acme/pages/{slug}.yaml"),
            text,
        );
    }
    for (slug, text) in QUERIES {
        write(
            dir.path(),
            &format!(".oxplow/extensions/acme/queries/{slug}.yaml"),
            text,
        );
    }
    // A second extension mounting its pages into acme's.
    write(
        dir.path(),
        ".oxplow/extensions/beta/extension.yaml",
        "manifest: 2\nname: beta\nintent:\n  purpose: test\nui:\n  mounts:\n    - { page: extra, at: acme/slotted.sections }\n    - { page: early, at: acme/slotted.sections, position: start }\n",
    );
    write(
        dir.path(),
        ".oxplow/extensions/beta/pages/extra.yaml",
        "title: Extra\nparams: [{ name: n }]\nloaders:\n  echo: { sql: \"SELECT :n AS got\" }\nview: { viz: number, from: echo }\n",
    );
    write(
        dir.path(),
        ".oxplow/extensions/beta/pages/early.yaml",
        "title: Early\nloaders:\n  e: { sql: \"SELECT 0 AS e\" }\nview: { viz: number, from: e }\n",
    );
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    let problems = crate::test_fixtures::activate_with_problems(&svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    (dir, svc)
}

pub(crate) async fn run(svc: &crate::Services, page: &str, params: &[(&str, SqlCell)]) -> PageRun {
    PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(page.into()),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            ctx: ViewContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap()
}
