//! The knowledge conformance suite (`.context/knowledge.md`): what
//! every knowledge store must do, checked through what a person and an
//! agent see — the `oxplow.knowledge.*` commands in, core's record
//! (`v_knowledge_page`, `v_knowledge_body`, the pins, the event log) out.
//! It runs in-tree against oxwiki and, through the conformance kit
//! (`oxplow extension test`), against a provider process, the same way:
//! core never special-cases its own store.
//!
//! Each check is a [`Finding`] when it fails; an empty list passes. The
//! suite moves the world by recording a snapshot of a file it changes
//! (standing in for the capture), so run it on a scratch project.

use std::sync::atomic::{AtomicU32, Ordering};

use oxplow_domain::Actor;
use serde_json::json;

pub use crate::conformance::Finding;

/// A page as core records it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRow {
    title: String,
    updated_at: String,
}

/// Run every check against the enabled store `store` as `actor` — one who
/// may delete (a person: an agent's destructive call is left for a person
/// to confirm). Every command names the store, so it runs beside others.
pub async fn suite(svc: &crate::Services, store: &str, actor: &Actor) -> Vec<Finding> {
    let world = World {
        svc,
        drifts: AtomicU32::new(0),
    };
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let file = "conformance/pinned.txt";
    world.drift(file).await;
    let body = format!("# Conformance\n\nRelies on [[{file}]].\n");
    let write = |body: &str, verified: &[&str]| {
        json!({
            "store": store,
            "id": "conformance-page",
            "title": "Conformance",
            "body": body,
            "verified_files": verified,
        })
    };
    let run = |input: serde_json::Value, name: &'static str| async move {
        svc.commands
            .run(actor, name, input, true)
            .await
            .map(|o| o.result)
    };

    // 1. A write lands the page, its body and its event.
    let page = match run(write(&body, &[]), crate::knowledge::WRITE_PAGE).await {
        Ok(result) => result["page"].as_str().unwrap_or_default().to_string(),
        Err(e) => {
            fail("write", format!("write failed: {e}"));
            return findings;
        }
    };
    world.settle().await;
    let first = world.page(&page).await;
    match &first {
        None => fail("write", format!("no row for `{page}`")),
        Some(row) if row.title != "Conformance" => {
            fail("write", format!("title is {:?}", row.title))
        }
        Some(_) => {}
    }
    if world.body(&page).await.as_deref() != Some(body.as_str()) {
        fail("write", "the recorded body isn't what was written".into());
    }
    if !world
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
    let freshness = || crate::knowledge::freshness(&svc.db, &page);
    match freshness().await {
        Ok(f) if stale(&f) == Some(false) => {}
        other => fail("freshness", format!("fresh after writing: {other:?}")),
    }
    world.drift(file).await;
    match freshness().await {
        Ok(f) if stale(&f) == Some(true) => {}
        other => fail(
            "freshness",
            format!("not stale after the file drifted: {other:?}"),
        ),
    }
    let edited = format!("{body}\nMore prose.\n");
    let _ = run(write(&edited, &[]), crate::knowledge::WRITE_PAGE).await;
    match freshness().await {
        Ok(f) if stale(&f) == Some(true) => {}
        other => fail(
            "freshness",
            format!("an unverified rewrite re-pinned the ref: {other:?}"),
        ),
    }
    world.settle().await;
    if world.page(&page).await.map(|r| r.updated_at) <= first.map(|r| r.updated_at) {
        fail("rewrite", "a rewrite didn't move updated_at".into());
    }
    let _ = run(write(&edited, &[file]), crate::knowledge::WRITE_PAGE).await;
    match freshness().await {
        Ok(f) if stale(&f) == Some(false) => {}
        other => fail("freshness", format!("verifying didn't re-pin: {other:?}")),
    }

    // 3. A dangling link is refused, naming it.
    match run(
        write("# X\n\n[[no-such-conformance-page]]\n", &[]),
        crate::knowledge::WRITE_PAGE,
    )
    .await
    {
        Ok(_) => fail("links", "a dangling link was accepted".into()),
        Err(e) if !e.to_string().contains("no-such-conformance-page") => {
            fail("links", format!("the refusal doesn't name the link: {e}"))
        }
        Err(_) => {}
    }

    if page != oxplow_domain::uri::build::knowledge_uri(store, "conformance-page") {
        fail("write", format!("the write named another page: `{page}`"));
    }

    // 4. A link lands in the page, through the store, naming the other page
    //    by its store.
    let other = "conformance-other";
    let _ = run(
        json!({ "store": store, "id": other, "body": "# Other\n" }),
        crate::knowledge::WRITE_PAGE,
    )
    .await;
    world.settle().await;
    let target = format!("{store}:{other}");
    match run(
        json!({ "store": store, "id": "conformance-page", "target": target }),
        crate::knowledge::LINK,
    )
    .await
    {
        Err(e) => fail("link", format!("link failed: {e}")),
        Ok(_) => {
            let linked = world
                .body(&page)
                .await
                .is_some_and(|b| b.contains(&format!("[[{target}]]")));
            if !linked {
                fail("link", format!("`{page}`'s body doesn't link `{other}`"));
            }
        }
    }

    // 5. A delete takes the page, and says so.
    for id in ["conformance-page", other] {
        if let Err(e) = run(
            json!({ "store": store, "id": id }),
            crate::knowledge::DELETE_PAGE,
        )
        .await
        {
            fail("delete", format!("delete of `{id}` failed: {e}"));
        }
    }
    world.settle().await;
    if world.page(&page).await.is_some() {
        fail("delete", format!("`{page}` is still there"));
    }
    if !world
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

fn sql(e: rusqlite::Error) -> oxplow_domain::DomainError {
    oxplow_domain::DomainError::Storage(e.to_string())
}

/// What the suite reads of core's record, and how it moves the world.
struct World<'a> {
    svc: &'a crate::Services,
    drifts: AtomicU32,
}

impl World<'_> {
    async fn settle(&self) {
        // Every consumer, the model record's too: a link to a page checks
        // the model's row of it.
        let _ = self.svc.event_pump.run_once().await;
        self.svc
            .event_pump
            .settle_all(std::time::Duration::from_secs(10))
            .await;
    }

    async fn page(&self, page: &str) -> Option<PageRow> {
        let page = page.to_string();
        self.svc
            .db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT title, updated_at FROM v_knowledge_page WHERE uri = ?1",
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
            .ok()
            .flatten()
    }

    async fn body(&self, page: &str) -> Option<String> {
        let page = page.to_string();
        self.svc
            .db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT body FROM v_knowledge_body WHERE uri = ?1",
                    [page],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)
            })
            .await
            .ok()
            .flatten()
    }

    /// The type of every logged event whose subject names `page`.
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
            .unwrap_or_default()
    }

    /// Change the workspace file at `path` (creating it) and record a
    /// snapshot of it on the primary stream, as the capture would.
    async fn drift(&self, path: &str) {
        let n = self.drifts.fetch_add(1, Ordering::SeqCst);
        let file = self.svc.layout.project_dir.join(path);
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, format!("version {n}\n"));
        let path = path.to_string();
        let _ = self
            .svc
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
            .await;
        // updated_at has millisecond resolution; keep writes apart.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}
