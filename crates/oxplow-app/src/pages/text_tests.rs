//! A page's text: what an agent reads (`run_page`, `get_open_page`, Copy):
//! each node under a heading naming its id, its rows as its viz renders
//! them, an embed's page nested inside.

use oxplow_db::SqlCell;

use super::test_support::{fixture, run};
use super::text::page_text;

#[tokio::test]
async fn each_node_reads_under_its_id_with_its_loaders_rows() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/numbers", &[("n", SqlCell::Int(2))]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(text.starts_with("# Numbers\n"), "{text}");
    assert!(text.contains("[node big]"), "{text}");
    assert!(text.contains("[node count]"), "{text}");
    // A table renders its rows; a number its value.
    assert!(text.contains("| 3 |"), "{text}");
    assert!(text.contains("\n2\n") || text.contains("\n2"), "{text}");
}

#[tokio::test]
async fn an_embed_reads_nested_with_its_ids_under_the_embeds() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/host", &[]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(text.contains("## Picked"), "{text}");
    assert!(text.contains("[node picked/echo]"), "{text}");
}

#[tokio::test]
async fn a_loader_that_failed_says_why_where_its_node_is() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/writes", &[]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(text.contains("isn't a read"), "{text}");
}
