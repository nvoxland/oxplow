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

/// A section's `description` reads as a line under its heading, so an
/// agent knows what the section holds, as the person does from its ⓘ.
#[tokio::test]
async fn a_section_description_reads_under_its_heading() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/numbers", &[("n", SqlCell::Int(2))]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(
        text.contains("## Big ones [node big]\n\nNumbers at or above n.\n\n"),
        "{text}"
    );
}

/// A section whose title isn't drawn (`hide_title`) still reads under its
/// heading, so an agent can name it.
#[tokio::test]
async fn a_section_with_a_hidden_title_still_has_its_heading() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/numbers", &[("n", SqlCell::Int(2))]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(text.contains("## Total [node count]"), "{text}");
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

/// "See also" ends the page's text, a `- [title](uri)` line per link;
/// a page with none has no such heading.
#[tokio::test]
async fn see_also_ends_the_text_and_is_absent_without_links() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/related", &[]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(
        text.ends_with(
            "\n## See also\n\n- [Numbers](oxplow:oxplow/page?n=3&page=acme/numbers)\n- [Docs](oxplow:oxplow/page?page=acme/numbers)\n"
        ),
        "{text}"
    );
    let r = run(&svc, "acme/numbers", &[("n", SqlCell::Int(2))]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(!text.contains("See also"), "{text}");
}

/// A node's `lead` is the first row's text, a paragraph under its heading and
/// above its viz; an empty one writes nothing.
#[tokio::test]
async fn a_lead_is_a_paragraph_under_the_heading_and_an_empty_one_is_nothing() {
    let (_dir, svc) = fixture().await;
    let r = run(&svc, "acme/led", &[]).await;
    let text = page_text(&svc, &svc.layout.project_dir, &r).await.unwrap();
    assert!(
        text.contains("## With [node with]\n\nThis is what it is.\n\n"),
        "{text}"
    );
    assert!(text.contains("## Without [node without]\n\n"), "{text}");
    assert_eq!(text.matches("This is what it is.").count(), 1, "{text}");
}
