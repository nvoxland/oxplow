//! Cores for the `page_refs` command module — unified cross-page
//! reference graph reader.
//!
//! Both directions of the edge are exposed:
//! - `list_backlinks(target)` — what points AT this page (its URI).
//!   Drives the Backlinks dropdown / panel for every page.
//! - `list_outbound(source)` — what this page points to. Drives the
//!   Outbound dropdown.
//!
//! The reader joins labels (wiki title, task title, commit subject) at
//! read time so the renderer doesn't need a second round-trip per row.
//! Labels are best-effort — when the other end is gone (e.g. a deleted
//! task) the label is `None` and the renderer falls back to its URI.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::Services;
use oxplow_db::PageRefEdge;

use crate::error::IpcError;

/// Edge plus a best-effort renderer label for its other end.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklinkEdge {
    /// The URI that refers.
    pub source: String,
    /// The URI it refers to.
    pub target: String,
    pub ref_type: String,
    pub source_extra: Option<String>,
    /// Human label for the other end (the source of a backlink, the
    /// target of an outbound edge): wiki title, task title, commit
    /// subject, …. The renderer falls back to the URI when None.
    pub source_label: Option<String>,
    /// Whether anything resolves the row the label belongs to. A link to
    /// a URI nothing resolves — an extension gone, a record deleted — is
    /// a dead link the renderer shows as its raw text.
    pub resolved: bool,
}

pub async fn list_backlinks(
    svc: &Services,
    target: String,
    limit: Option<u32>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = svc
        .page_ref_store
        .list_backlinks(&target, limit.map(i64::from))
        .await?;
    Ok(decorate(svc, edges, |e| &e.source).await)
}

pub async fn list_outbound(
    svc: &Services,
    source: String,
    limit: Option<u32>,
) -> Result<Vec<BacklinkEdge>, IpcError> {
    let edges = svc
        .page_ref_store
        .list_outbound(&source, limit.map(i64::from))
        .await?;
    // For outbound, the label is the *target's*, in the same field, so
    // the renderer stays resource-agnostic. A target with no first-class
    // label (a file, a directory) leaves it None — the renderer shows
    // its path, the meaningful display for those.
    Ok(decorate(svc, edges, |e| &e.target).await)
}

/// Each edge with the label of the end `labelled` picks.
async fn decorate(
    svc: &Services,
    edges: Vec<PageRefEdge>,
    labelled: fn(&PageRefEdge) -> &String,
) -> Vec<BacklinkEdge> {
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let summary = oxplow_app::ref_resolver::resolve_ref(svc, labelled(&e)).await;
        out.push(BacklinkEdge {
            source: e.source,
            target: e.target,
            ref_type: e.ref_type,
            source_extra: e.source_extra,
            source_label: summary.title,
            resolved: summary.resolved,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_backlinks_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "list_backlinks",
            serde_json::json!({ "target": "oxplow:oxplow/wiki?slug=some-slug" }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_outbound_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "list_outbound",
            serde_json::json!({ "source": "oxplow:oxplow/wiki?slug=some-slug", "limit": 5 }),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    /// An edge's `resolved` says whether its far end is a ref anything
    /// names: a link to a collector nothing installed defines comes back
    /// unresolved with no label, never an error.
    #[tokio::test]
    async fn an_outbound_edge_says_whether_its_target_resolves() {
        let (svc, _dir) = crate::test_support::services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .selected_or_first(&stream.id)
            .await
            .unwrap()
            .expect("the primary stream has a thread");
        let made = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                serde_json::json!({
                    "title": "Fix it",
                    "thread": oxplow_domain::uri::build::thread_uri(thread),
                    "links": [{ "target": "oxplow:oxplow/collector?collector=ghost/none", "link_type": "about" }],
                }),
                false,
            )
            .await
            .unwrap();
        let item = made.result["ref"].as_str().unwrap().to_string();
        let out = crate::dispatch("list_outbound", serde_json::json!({ "source": item }), &svc)
            .await
            .unwrap();
        let edge = out
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                oxplow_domain::uri::build::resource_of(e["target"].as_str().unwrap()).as_deref()
                    == Some("oxplow/collector")
            })
            .unwrap_or_else(|| panic!("no collector edge in {out}"));
        assert_eq!(edge["resolved"], false);
        assert!(edge["source_label"].is_null());
    }
}
