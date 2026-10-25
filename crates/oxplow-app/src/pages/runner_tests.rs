//! Running a page: its loaders in order (temp tables, commands, other
//! pages, inline rows), its tree (a switch's child in view, embeds given
//! params by name), and which loaders a change makes run again.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::runner::*;
use crate::extensions::LensContext;

fn write(root: &Path, rel: &str, text: &str) {
    let file = root.join(rel);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

const PAGES: &[(&str, &str)] = &[
    (
        "numbers",
        r#"
title: Numbers
params: [{ name: n, default: 2 }]
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

async fn fixture() -> (tempfile::TempDir, Arc<crate::Services>) {
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
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    let problems = crate::test_fixtures::activate_with_problems(&svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    (dir, svc)
}

async fn run(svc: &crate::Services, page: &str, params: &[(&str, SqlCell)]) -> PageRun {
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

fn ints(run: &PageRun, loader: &str) -> Vec<i64> {
    run.loaders[loader]
        .result
        .rows
        .iter()
        .map(|r| match &r[0] {
            SqlCell::Int(i) => *i,
            other => panic!("{other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn loaders_run_in_order_and_later_ones_read_earlier_ones() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/numbers", &[]).await;
    assert_eq!(ints(&r, "big"), [2, 3]);
    assert_eq!(ints(&r, "count"), [2]);
    let r = run(&svc, "acme/numbers", &[("n", SqlCell::Int(3))]).await;
    assert_eq!(ints(&r, "big"), [3]);
    assert_eq!(r.params.get("n"), Some(&SqlCell::Int(3)));
    // The tree mirrors the view.
    assert_eq!(r.tree.id, "stack");
    let ids: Vec<&str> = r.tree.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["big", "count"]);
}

#[tokio::test]
async fn an_embed_takes_params_by_name_from_the_hosts_loaders() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/host", &[]).await;
    let embed = r.tree.children[0].embed.as_ref().expect("the embed ran");
    assert_eq!(embed.page.id, "acme/picked");
    assert_eq!(ints(embed, "echo"), [3], "`total` came from loader `first`");
    // Another page's loader, run with the params it was given.
    assert_eq!(ints(&r, "borrowed"), [3]);
    // Inline rows.
    assert_eq!(r.loaders["colors"].result.columns, ["state"]);
    assert_eq!(r.loaders["colors"].result.rows.len(), 2);
}

#[tokio::test]
async fn a_switch_runs_only_the_child_in_view() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/areas", &[]).await;
    let ids: Vec<&str> = r.tree.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["one"]);
    let r = run(&svc, "acme/areas", &[("area", SqlCell::Text("two".into()))]).await;
    let ids: Vec<&str> = r.tree.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["two"]);
    assert!(
        !r.loaders.contains_key("one"),
        "a loader only the hidden child shows doesn't run"
    );
}

#[tokio::test]
async fn a_command_loader_runs_a_read_command_and_refuses_any_other() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/queried", &[]).await;
    assert_eq!(r.loaders["q"].error, None);
    assert_eq!(r.loaders["q"].result.columns, ["seven"]);
    assert_eq!(ints(&r, "q"), [7]);
    let r = run(&svc, "acme/writes", &[]).await;
    let err = r.loaders["w"].error.as_deref().unwrap_or_default();
    assert!(err.contains("isn't a read"), "{err}");
}

#[tokio::test]
async fn an_unknown_param_is_refused() {
    let (_dir, svc) = fixture().await;
    let err = PageRunner::of(&svc)
        .run(RunRequest {
            target: RunTarget::Page("acme/numbers".into()),
            params: BTreeMap::from([("nope".to_string(), SqlCell::Int(1))]),
            ctx: LensContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
}

/// What a change makes run again: the loaders that read a changed model
/// or bind a changed param, and every loader downstream of them.
#[tokio::test]
async fn a_change_dirties_what_reads_it_and_what_follows() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/numbers", &[]).await;
    let page = &r.page;
    let none: [String; 0] = [];
    assert_eq!(
        dirty_loaders(page, &r, &none, &["n".to_string()]),
        ["big", "count"]
    );
    assert!(dirty_loaders(page, &r, &["v_commit".to_string()], &none).is_empty());
    let r = run(&svc, "acme/queried", &[]).await;
    assert_eq!(
        dirty_loaders(&r.page, &r, &["v_model".to_string()], &none),
        ["q"]
    );
}
