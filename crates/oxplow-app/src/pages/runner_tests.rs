//! Running a page: its loaders in order (temp tables, commands, other
//! pages, inline rows), its tree (a switch's child in view, embeds given
//! params by name), and which loaders a change makes run again.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::runner::*;
use super::test_support::{fixture, run};
use crate::extensions::LensContext;

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
