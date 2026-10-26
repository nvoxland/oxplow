//! Running a page: its loaders in order (temp tables, commands, other
//! pages, inline rows), its tree (a switch's child in view, embeds given
//! params by name), and which loaders a change makes run again.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::runner::*;
use super::test_support::{fixture, run};
use crate::extensions::ViewContext;

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

/// `items:` takes a list inside a command's result as the rows: the
/// definition's `nodes`, one row per node.
#[tokio::test]
async fn a_command_loaders_items_are_a_list_in_its_result() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/listed", &[]).await;
    assert_eq!(r.loaders["nodes"].error, None);
    let result = &r.loaders["nodes"].result;
    let id = result
        .columns
        .iter()
        .position(|c| c == "id")
        .expect("an id column");
    let ids: Vec<String> = result
        .rows
        .iter()
        .map(|row| format!("{:?}", row[id]))
        .collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    let err = r.loaders["missing"].error.as_deref().unwrap_or_default();
    assert!(err.contains("returned no `nope`"), "{err}");
}

/// A page resolves `head_sha` itself from the viewed stream: a change to
/// `v_stream` (a commit, a branch switch) reaches the loaders that bind
/// it, and only those.
#[tokio::test]
async fn a_stream_change_reaches_what_binds_head_sha() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/headed", &[]).await;
    let kept = unreached(&r, &["v_stream".to_string()], &[]);
    let keys: Vec<&String> = kept.keys().collect();
    assert_eq!(keys, [&r.loaders["other"].key], "only `at` re-runs");
}

#[tokio::test]
async fn an_unknown_param_is_refused() {
    let (_dir, svc) = fixture().await;
    let err = PageRunner::of(&svc)
        .run(RunRequest {
            target: RunTarget::Page("acme/numbers".into()),
            params: BTreeMap::from([("nope".to_string(), SqlCell::Int(1))]),
            ctx: ViewContext::default(),
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

#[tokio::test]
async fn a_query_binds_an_earlier_loaders_column_by_name() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/bound", &[]).await;
    assert_eq!(r.loaders["echo"].error, None);
    assert_eq!(ints(&r, "echo"), [42]);
}

/// Another extension's pages mounted at a slot run as embeds there, given
/// the host's params by name; `at: start` puts one first among the slot's
/// siblings.
#[tokio::test]
async fn a_slot_runs_the_pages_mounted_at_it() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/slotted", &[]).await;
    let ids: Vec<&str> = r.tree.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        ["beta.early", "first", "sections", "last"],
        "{:?}",
        r.tree
    );
    let slot = &r.tree.children[2];
    let mounted: Vec<&str> = slot.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(mounted, ["beta.extra"]);
    let extra = slot.children[0].embed.as_ref().expect("it ran");
    assert_eq!(extra.page.id, "beta/extra");
    assert_eq!(ints(extra, "echo"), [4], "`n` flowed from the host");
}

fn budget_page(slug: &str, yaml: &str) -> super::schema::Page {
    super::schema::parse_page("acme", "acme", slug, "budget-test.yaml", yaml).unwrap()
}

fn result_bytes(run: &PageRun) -> usize {
    fn nodes(node: &RunNode) -> usize {
        node.embed.as_deref().map(result_bytes).unwrap_or(0)
            + node.children.iter().map(nodes).sum::<usize>()
    }
    run.loaders
        .values()
        // Every admitted result is bounded, so this test-only serialization
        // cannot allocate an arbitrary loader's original output.
        .map(|loader| serde_json::to_vec(&loader.result).unwrap().len())
        .sum::<usize>()
        + nodes(&run.tree)
        + run.rail.as_ref().map(nodes).unwrap_or(0)
}

#[tokio::test]
async fn the_page_result_budget_omits_an_oversized_single_cell_and_marks_truncated() {
    let (_dir, svc) = fixture().await;
    let page = budget_page(
        "huge-cell",
        "title: Huge cell\ndata:\n  huge: { sql: \"SELECT hex(zeroblob(3000000)) AS payload\" }\nview: { viz: table, from: huge }\n",
    );
    let run = PageRunner::of(&svc)
        .run(RunRequest {
            target: RunTarget::Spec(std::sync::Arc::new(page)),
            params: BTreeMap::new(),
            ctx: ViewContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap();
    let loader = &run.loaders["huge"];
    assert!(loader.error.is_none(), "{:?}", loader.error);
    assert_eq!(loader.result.columns, ["payload"]);
    assert!(loader.result.rows.is_empty());
    assert!(loader.result.truncated);
    assert!(result_bytes(&run) <= MAX_RESULT_BYTES);
}

#[tokio::test]
async fn downstream_loaders_read_only_rows_admitted_to_the_page_result_budget() {
    let (_dir, svc) = fixture().await;
    let page = budget_page(
        "cropped-input",
        "title: Cropped input\ndata:\n  huge:\n    sql: \"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < 8) SELECT x, hex(zeroblob(500000)) AS payload FROM n\"\n  count: { sql: \"SELECT count(*) AS n FROM huge\" }\nview: { viz: number, from: count }\n",
    );
    let run = PageRunner::of(&svc)
        .run(RunRequest {
            target: RunTarget::Spec(std::sync::Arc::new(page)),
            params: BTreeMap::new(),
            ctx: ViewContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap();
    assert!(run.loaders["huge"].error.is_none());
    assert!(run.loaders["count"].error.is_none());
    assert!(run.loaders["huge"].result.truncated);
    assert_eq!(run.loaders["huge"].result.rows.len(), 5);
    assert_eq!(ints(&run, "count"), [5]);
    assert!(result_bytes(&run) <= MAX_RESULT_BYTES);
}

#[tokio::test]
async fn the_page_result_budget_is_shared_by_embeds_including_memo_copies_on_rerun() {
    let (dir, svc) = fixture().await;
    let child = budget_page(
        "large-child",
        "title: Large child\ndata:\n  payload: { sql: \"SELECT hex(zeroblob(1500000)) AS payload\" }\nview: { viz: table, from: payload }\n",
    );
    let page = budget_page(
        "repeated-embeds",
        "title: Repeated embeds\nview:\n  stack:\n  - { id: first, embed: { page: acme/large-child } }\n  - { id: second, embed: { page: acme/large-child } }\nrail: { id: rail-copy, embed: { page: acme/large-child } }\n",
    );
    let mut runner = PageRunner::of(&svc);
    runner.draft = Some(Draft {
        extension: "acme".into(),
        checkout: dir.path().to_path_buf(),
        pages: vec![child],
    });
    let request = RunRequest {
        target: RunTarget::Spec(std::sync::Arc::new(page)),
        params: BTreeMap::new(),
        ctx: ViewContext::default(),
        viewer: Actor::Human,
    };
    let run = runner.run(request.clone()).await.unwrap();
    let first = run.tree.children[0].embed.as_ref().unwrap();
    let second = run.tree.children[1].embed.as_ref().unwrap();
    let rail = run.rail.as_ref().unwrap().embed.as_ref().unwrap();
    assert_eq!(first.loaders["payload"].result.rows.len(), 1);
    assert!(!first.loaders["payload"].result.truncated);
    assert!(second.loaders["payload"].result.rows.is_empty());
    assert!(second.loaders["payload"].result.truncated);
    assert!(rail.loaders["payload"].result.rows.is_empty());
    assert!(rail.loaders["payload"].result.truncated);
    assert_ne!(first.loaders["payload"].key, second.loaders["payload"].key);
    assert!(result_bytes(&run) <= MAX_RESULT_BYTES);
    let seed = unreached(&run, &[], &[]);
    let rerun = runner.run_seeded(request, seed).await.unwrap();
    assert_eq!(run, rerun);
    assert!(result_bytes(&rerun) <= MAX_RESULT_BYTES);
}

#[tokio::test]
async fn a_page_loader_does_not_double_charge_its_transient_source_page_results() {
    let (dir, svc) = fixture().await;
    let child = budget_page(
        "large-source",
        "title: Large source\ndata:\n  payload: { sql: \"SELECT hex(zeroblob(2000000)) AS payload\" }\nview: { viz: table, from: payload }\n",
    );
    let page = budget_page(
        "borrow-large",
        "title: Borrow large\ndata:\n  borrowed: { page: acme/large-source, loader: payload }\nview: { viz: table, from: borrowed }\n",
    );
    let mut runner = PageRunner::of(&svc);
    runner.draft = Some(Draft {
        extension: "acme".into(),
        checkout: dir.path().to_path_buf(),
        pages: vec![child],
    });
    let run = runner
        .run(RunRequest {
            target: RunTarget::Spec(std::sync::Arc::new(page)),
            params: BTreeMap::new(),
            ctx: ViewContext::default(),
            viewer: Actor::Human,
        })
        .await
        .unwrap();
    let borrowed = &run.loaders["borrowed"];
    assert!(borrowed.error.is_none(), "{:?}", borrowed.error);
    assert_eq!(borrowed.result.rows.len(), 1);
    assert!(!borrowed.result.truncated);
    assert!(result_bytes(&run) <= MAX_RESULT_BYTES);
}
