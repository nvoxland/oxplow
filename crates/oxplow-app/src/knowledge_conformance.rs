//! The knowledge conformance suite (P5.C4, `.context/knowledge.md`): what
//! every [`KnowledgeProvider`] must do, as plain functions over the trait
//! and a [`KnowledgeProbe`] that reads what the host recorded and moves
//! the world (a file changing and being captured). It runs in-tree
//! against oxplow's wiki, and against an external provider through the
//! host (P5.D).
//!
//! Each check is a [`Finding`] when it fails; an empty list passes.

use async_trait::async_trait;
use oxplow_domain::knowledge::{KnowledgeProvider, PageDraft};
use oxplow_domain::Actor;

pub use crate::work_items_conformance::Finding;

/// A page as the host records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRow {
    pub title: String,
    pub updated_at: String,
}

/// What the suite reads back from, and does to, the host.
#[async_trait]
pub trait KnowledgeProbe: Send + Sync {
    async fn settle(&self);
    async fn page(&self, page: &str) -> Option<PageRow>;
    /// The page's body as the provider stores it.
    async fn body(&self, page: &str) -> Option<String>;
    /// The type of every logged event whose subject names `page`.
    async fn event_types(&self, page: &str) -> Vec<String>;
    /// Change the workspace file at `path` (creating it) and capture it.
    async fn drift(&self, path: &str);
}

/// Run every check against `provider` as `actor` — one who may delete
/// (a person: an agent's destructive call is left for a person to
/// confirm).
pub async fn suite(
    provider: &dyn KnowledgeProvider,
    probe: &dyn KnowledgeProbe,
    actor: &Actor,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let file = "conformance/pinned.txt";
    probe.drift(file).await;
    let body = format!("# Conformance\n\nRelies on [[{file}]].\n");
    let draft = |body: &str, verified: &[&str]| PageDraft {
        slug: "conformance-page".into(),
        title: None,
        body: body.to_string(),
        verified_refs: verified.iter().map(|s| s.to_string()).collect(),
        removed_refs: Vec::new(),
    };

    // 1. A write lands the page, its body and its event.
    let page = match provider.write_page(actor, draft(&body, &[])).await {
        Ok(page) => page,
        Err(e) => {
            fail("write", format!("write failed: {e}"));
            return findings;
        }
    };
    probe.settle().await;
    let first = probe.page(&page).await;
    match &first {
        None => fail("write", format!("no row for `{page}`")),
        Some(row) if row.title != "Conformance" => {
            fail("write", format!("title is {:?}", row.title))
        }
        Some(_) => {}
    }
    if probe.body(&page).await.as_deref() != Some(body.as_str()) {
        fail("write", "the stored body isn't what was written".into());
    }
    if !probe
        .event_types(&page)
        .await
        .iter()
        .any(|t| t == "knowledge.page.written")
    {
        fail(
            "write",
            format!("no `knowledge.page.written` names `{page}`"),
        );
    }

    // 2. Freshness: stale once the pinned file drifts; a rewrite that
    //    doesn't verify it keeps the old pin; verifying re-pins it.
    let stale = |f: &[oxplow_domain::knowledge::RefFreshness]| {
        f.iter().find(|r| r.target.ends_with(file)).map(|r| r.stale)
    };
    match provider.freshness(&page).await {
        Ok(f) if stale(&f) == Some(false) => {}
        other => fail("freshness", format!("fresh after writing: {other:?}")),
    }
    probe.drift(file).await;
    match provider.freshness(&page).await {
        Ok(f) if stale(&f) == Some(true) => {}
        other => fail(
            "freshness",
            format!("not stale after the file drifted: {other:?}"),
        ),
    }
    let edited = format!("{body}\nMore prose.\n");
    let _ = provider.write_page(actor, draft(&edited, &[])).await;
    match provider.freshness(&page).await {
        Ok(f) if stale(&f) == Some(true) => {}
        other => fail(
            "freshness",
            format!("an unverified rewrite re-pinned the ref: {other:?}"),
        ),
    }
    probe.settle().await;
    if probe.page(&page).await.map(|r| r.updated_at) <= first.map(|r| r.updated_at) {
        fail("rewrite", "a rewrite didn't move updated_at".into());
    }
    let _ = provider.write_page(actor, draft(&edited, &[file])).await;
    match provider.freshness(&page).await {
        Ok(f) if stale(&f) == Some(false) => {}
        other => fail("freshness", format!("verifying didn't re-pin: {other:?}")),
    }

    // 3. A dangling link is refused, naming it.
    match provider
        .write_page(actor, draft("# X\n\n[[no-such-conformance-page]]\n", &[]))
        .await
    {
        Ok(_) => fail("links", "a dangling link was accepted".into()),
        Err(e) if !e.to_string().contains("no-such-conformance-page") => {
            fail("links", format!("the refusal doesn't name the link: {e}"))
        }
        Err(_) => {}
    }

    // 4. A delete takes the page, and says so.
    if let Err(e) = provider.delete_page(actor, &page).await {
        fail("delete", format!("delete failed: {e}"));
    }
    probe.settle().await;
    if probe.page(&page).await.is_some() {
        fail("delete", format!("`{page}` is still there"));
    }
    if !probe
        .event_types(&page)
        .await
        .iter()
        .any(|t| t == "knowledge.page.deleted")
    {
        fail(
            "delete",
            format!("no `knowledge.page.deleted` names `{page}`"),
        );
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// [`KnowledgeProbe`] over the app, capturing by writing the file and a
    /// snapshot row for it (the fixture has no capture service running).
    struct AppProbe<'a> {
        svc: &'a crate::Services,
        drifts: AtomicU32,
    }

    fn sql(e: rusqlite::Error) -> oxplow_domain::DomainError {
        oxplow_domain::DomainError::Storage(e.to_string())
    }

    #[async_trait]
    impl KnowledgeProbe for AppProbe<'_> {
        async fn settle(&self) {
            self.svc.event_pump.run_once().await.unwrap();
        }

        async fn page(&self, page: &str) -> Option<PageRow> {
            let page = page.to_string();
            self.svc
                .db
                .read(move |c| {
                    use rusqlite::OptionalExtension;
                    c.query_row(
                        "SELECT title, updated_at FROM v_knowledge_page WHERE ref = ?1",
                        [page],
                        |r| {
                            Ok(PageRow {
                                title: r.get(0)?,
                                updated_at: r.get(1)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(sql)
                })
                .await
                .unwrap()
        }

        async fn body(&self, page: &str) -> Option<String> {
            let slug = page.strip_prefix("wiki:")?;
            std::fs::read_to_string(crate::knowledge::page_path(
                &self.svc.layout.project_dir,
                slug,
            ))
            .ok()
        }

        async fn event_types(&self, page: &str) -> Vec<String> {
            let page = page.to_string();
            self.svc
                .db
                .read(move |c| {
                    let mut stmt = c
                        .prepare(
                            "SELECT e.type FROM event_log e, json_each(e.subject) s
                             WHERE s.value = ?1 ORDER BY e.seq",
                        )
                        .map_err(sql)?;
                    let rows = stmt
                        .query_map([page], |r| r.get::<_, String>(0))
                        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                        .map_err(sql)?;
                    Ok(rows)
                })
                .await
                .unwrap()
        }

        async fn drift(&self, path: &str) {
            let n = self.drifts.fetch_add(1, Ordering::SeqCst);
            let file = self.svc.layout.project_dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, format!("version {n}\n")).unwrap();
            let path = path.to_string();
            self.svc
                .db
                .transaction(move |c| {
                    c.execute(
                        "INSERT INTO snapshot (stream_id, created_at)
                         SELECT id, '2026-09-30T00:00:00.000000Z' FROM streams WHERE kind = 'primary'",
                        [],
                    )
                    .map_err(sql)?;
                    let id = c.last_insert_rowid();
                    c.execute(
                        "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                         SELECT id, ?1, 'h', 1, '2026-09-30T00:00:00.000000Z', 'oxplow', ?2
                         FROM streams WHERE kind = 'primary'",
                        rusqlite::params![path, id],
                    )
                    .map_err(sql)?;
                    Ok(())
                })
                .await
                .unwrap();
            // updated_at has millisecond resolution; keep writes apart.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }

    /// oxplow's wiki passes the suite.
    #[tokio::test]
    async fn the_oxplow_wiki_is_a_conforming_provider() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let probe = AppProbe {
            svc: &fx.svc,
            drifts: AtomicU32::new(0),
        };
        // As a person: a delete is destructive, and an agent's is left for
        // the person to confirm.
        let findings = suite(&*fx.svc.knowledge, &probe, &Actor::Human).await;
        assert_eq!(findings, vec![]);
    }
}
