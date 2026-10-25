//! A project with extension `acme` holding the pages the page tests run.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::runner::*;
use crate::extensions::LensContext;

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
data:
  base: { sql: "SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3" }
  big: { sql: "SELECT n FROM base WHERE n >= :n ORDER BY n" }
  count: { sql: "SELECT count(*) AS total FROM big" }
view:
  stack:
    - { viz: table, from: big }
    - { viz: number, from: count }
"#,
    ),
    (
        "picked",
        r#"
title: Picked
params: [{ name: total }]
data:
  echo: { sql: "SELECT :total AS got" }
view: { viz: number, from: echo }
"#,
    ),
    (
        "host",
        r#"
title: Host
data:
  first: { sql: "SELECT 3 AS total" }
  borrowed: { page: acme/numbers, loader: count, params: { n: 1 } }
  colors: { rows: [{ state: failed }, { state: passed }] }
view:
  stack:
    - { embed: { page: acme/picked } }
    - { viz: number, from: borrowed }
    - { viz: list, from: colors }
"#,
    ),
    (
        "areas",
        r#"
title: Areas
params: [{ name: area, role: filter, default: one }]
data:
  one: { sql: "SELECT 'one' AS a" }
  two: { sql: "SELECT 'two' AS a" }
view:
  switch:
    param: area
    children:
      - { id: one, viz: list, from: one }
      - { id: two, viz: list, from: two }
"#,
    ),
    (
        "queried",
        r#"
title: Queried
data:
  q:
    command: oxplow.sql.query
    args: { sql: "SELECT 7 AS seven" }
    reads: [v_model]
view: { viz: table, from: q }
"#,
    ),
    (
        "listed",
        r#"
title: Listed
data:
  nodes:
    command: oxplow.page.definition
    args: { page: acme/numbers }
    items: nodes
    reads: [v_page]
  missing:
    command: oxplow.page.definition
    args: { page: acme/numbers }
    items: nope
    reads: [v_page]
view: { viz: table, from: nodes }
"#,
    ),
    (
        "bound",
        r#"
title: Bound
data:
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
data:
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
data:
  a: { sql: "SELECT :sha AS s" }
view: { viz: number, from: a }
"#,
    ),
    (
        "session",
        r#"
title: Session
data:
  row: { sql: "SELECT 'sess-1' AS session, 1 AS running" }
  other: { sql: "SELECT 1 AS n UNION ALL SELECT 2" }
actions:
  - { id: stop, label: Stop, command: oxplow.work_item.create, input: { title: "stop {{row.session}}" }, when: running }
view: { viz: table, from: other }
"#,
    ),
    (
        "new-item",
        r#"
title: New Item
params: [{ name: t, default: drafted }]
data:
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
data:
  first: { sql: "SELECT 1 AS a" }
  last: { sql: "SELECT 2 AS b" }
view:
  stack:
    - { viz: number, from: first }
    - { slot: sections }
    - { viz: number, from: last }
"#,
    ),
    (
        "mixed",
        r#"
title: Mixed
data:
  steady: { sql: "SELECT abs(random()) AS r" }
  models: { sql: "SELECT count(*) AS n FROM v_model" }
  after: { sql: "SELECT n, abs(random()) AS r FROM models" }
view:
  stack:
    - { viz: number, from: steady }
    - { viz: table, from: after }
"#,
    ),
    (
        "writes",
        r#"
title: Writes
data:
  w:
    command: oxplow.work_item.create
    args: { title: nope }
    refresh: manual
view: { viz: table, from: w }
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
    // A second extension mounting its pages into acme's.
    write(
        dir.path(),
        ".oxplow/extensions/beta/extension.yaml",
        "manifest: 2\nname: beta\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: acme/slotted.sections, page: extra }\n    - { slot: acme/slotted.sections, page: early, at: start }\n",
    );
    write(
        dir.path(),
        ".oxplow/extensions/beta/pages/extra.yaml",
        "title: Extra\nparams: [{ name: n }]\ndata:\n  echo: { sql: \"SELECT :n AS got\" }\nview: { viz: number, from: echo }\n",
    );
    write(
        dir.path(),
        ".oxplow/extensions/beta/pages/early.yaml",
        "title: Early\ndata:\n  e: { sql: \"SELECT 0 AS e\" }\nview: { viz: number, from: e }\n",
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
            ctx: LensContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap()
}
