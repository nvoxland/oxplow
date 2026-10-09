//! Knowledge — the wiki — as commands (P5.C3, `.context/knowledge.md`).
//!
//! `oxplow.knowledge.write_page` is the one way a page is written: in the bus's
//! transaction it validates the slug and every `[[link]]`, restates the
//! `wiki_page` row and its `page_ref` edges (new file refs pinned to the
//! primary stream's latest snapshot) and logs `knowledge.page.written`;
//! once committed it writes `.oxplow/wiki/<slug>.md`. Beside it:
//! `oxplow.knowledge.delete_page`, `oxplow.knowledge.link` (add a link to a page) and
//! `oxplow.knowledge.resync` (restate a page from its file — the repair op).
//! A hand edit of the file converges through the same core
//! ([`crate::wiki_pages::sync_page`], logged as `system:wiki_watch`).

use crate::commands::ops::Op;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::page_ref_projections::{
    stamp_file_versions, wiki_edges, KIND_FILE, KIND_WIKI, RT_WIKI_FILE,
};
use oxplow_db::page_ref_store::{merge_source_tx, replace_source_tx, upsert_edge_tx};
use oxplow_db::{EventCtx, FileRefVersion, PageRefEdge, WikiPage};
use oxplow_domain::events::schema::{
    KnowledgePageDeleted, KnowledgePageDeletedV1, KnowledgePageWritten, KnowledgePageWrittenV1,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::knowledge::{Kept, KnowledgeProvider, PageCall, PageDraft, RefFreshness};
use oxplow_domain::vcs::{Revision, Vcs};
use oxplow_domain::{Anchors, CommandError, Confirm, DomainError, Timestamp};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::util::{invalid, parse, schema};
use crate::commands::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::link_check::{check_links_in, LinkWorld};
use crate::wiki_pages::{
    extract_title, parse_refs, path_under_any_dir, strip_body_version_literals, wiki_pages_dir,
};

/// The capability.
pub const CAPABILITY: &str = "knowledge";
/// The wiki's built-in entry (`capabilities::BUILT_INS`), which
/// `oxplow-bundled` declares.
pub const BUILT_IN: &str = "oxplow:wiki";

pub const WRITE_PAGE: &str = "oxplow.knowledge.write_page";
pub const DELETE_PAGE: &str = "oxplow.knowledge.delete_page";
pub const LINK: &str = "oxplow.knowledge.link";
pub const RESYNC: &str = "oxplow.knowledge.resync";

/// A page's ref: `wiki:<slug>`.
pub fn page_ref(slug: &str) -> String {
    format!("{KIND_WIKI}:{slug}")
}

/// Where a page's body lives.
pub fn page_path(project_dir: &Path, slug: &str) -> PathBuf {
    wiki_pages_dir(project_dir).join(format!("{slug}.md"))
}

/// A slug is kebab-case: lowercase letters and digits in `-`-joined runs.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// What new file refs are pinned to: the primary stream's latest snapshot,
/// and the revision that snapshot is exactly, when it is one. Freshness
/// compares the pinned snapshot to a file's latest (`wiki_ref_drift`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pin {
    pub snapshot_id: Option<i64>,
    pub revision: Option<String>,
}

pub fn pin_tx(conn: &rusqlite::Connection) -> Result<Pin, DomainError> {
    let row: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT s.id, s.revision FROM snapshot s
             JOIN streams st ON st.id = s.stream_id AND st.kind = 'primary'
             ORDER BY s.id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    Ok(match row {
        None => Pin::default(),
        Some((id, revision)) => Pin {
            snapshot_id: Some(id),
            revision: revision
                .and_then(|r| r.parse::<Revision>().ok())
                .and_then(|r| r.vcs_rev().map(str::to_string)),
        },
    })
}

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}

impl Pin {
    fn stamp(&self, edge: PageRefEdge) -> PageRefEdge {
        match self.snapshot_id {
            Some(id) => edge.with_version(id, self.revision.clone(), self.revision.is_some()),
            None => edge,
        }
    }
}

/// A page to restate from `body`, with the refs its author re-checked
/// (`verified`: pinned to now) and dropped (`removed`: must be gone from
/// the body).
pub struct PageWrite<'a> {
    pub slug: &'a str,
    pub body: &'a str,
    pub verified: &'a [String],
    pub removed: &'a [String],
    pub updated_at: Timestamp,
    /// Who wrote it, on the event (a command's actor's thread and
    /// stream; none for the watcher).
    pub anchors: Anchors,
}

/// What a write left: the page's outbound refs and its pin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Written {
    pub page: String,
    pub outbound: Vec<String>,
    pub snapshot: Option<String>,
}

/// Refuse `verified` / `removed` refs the body doesn't bear out: a removed
/// ref still cited, a verified one neither cited nor under a cited
/// directory.
pub fn refs_borne_out(
    body: &str,
    verified: &[String],
    removed: &[String],
) -> Result<(), CommandError> {
    let refs = parse_refs(body);
    let still_cited: Vec<&str> = removed
        .iter()
        .filter(|p| refs.file_refs.contains(p))
        .map(String::as_str)
        .collect();
    if !still_cited.is_empty() {
        return Err(invalid(
            "/removed_refs",
            format!(
                "removed_refs still cited by the body: {}",
                still_cited.join(", ")
            ),
        ));
    }
    let unreferenced: Vec<&str> = verified
        .iter()
        .filter(|p| !refs.file_refs.contains(p) && !path_under_any_dir(p, &refs.dir_refs))
        .map(String::as_str)
        .collect();
    if !unreferenced.is_empty() {
        return Err(invalid(
            "/verified_refs",
            format!(
                "verified_refs neither cited by the body nor under a cited directory: {}",
                unreferenced.join(", ")
            ),
        ));
    }
    Ok(())
}

/// Restate a page's row and edges from its body, answering what it left
/// and its `knowledge.page.written` for the caller to log. Edges already stored keep their pins (so
/// unrelated prose edits don't re-stamp them); new file refs and verified
/// ones take the current [`Pin`]. A file verified under a directory the
/// body cites (`[[dir:…]]`) becomes an edge of its own, kept while that
/// directory stays cited.
pub fn write_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    project_dir: &Path,
    write: &PageWrite<'_>,
) -> Result<(Written, Envelope), DomainError> {
    let PageWrite {
        slug,
        body,
        verified,
        removed,
        updated_at,
        ref anchors,
    } = *write;
    let refs = parse_refs(body);
    refs_borne_out(body, verified, removed).map_err(|e| DomainError::Invalid(e.to_string()))?;
    let created_at = oxplow_db::wiki_page_store::get_tx(conn, slug)?
        .map(|(page, _)| page.created_at)
        .unwrap_or(updated_at);
    let page = WikiPage {
        slug: slug.to_string(),
        title: extract_title(body, slug),
        body_path: page_path(project_dir, slug).to_string_lossy().into_owned(),
        body_excerpt: body.chars().take(280).collect(),
        body_size_bytes: body.len() as i64,
        file_refs: refs.file_refs.clone(),
        dir_refs: refs.dir_refs.clone(),
        related_notes: refs.related_notes,
        created_at,
        updated_at,
    };
    oxplow_db::wiki_page_store::upsert_tx(conn, &page, body, &body_hash(body))?;

    let pin = pin_tx(conn)?;
    let mut edges = wiki_edges(&ev.vocabulary.kinds, slug, body);
    if let Some(id) = pin.snapshot_id {
        stamp_file_versions(
            &mut edges,
            FileRefVersion {
                local_snapshot_id: id,
                closest_vcs_rev: pin.revision.as_deref(),
                vcs_rev_exact: pin.revision.is_some(),
            },
        );
    }
    // Verification edges under a still-cited directory stay (with their
    // pins); merge_source prunes the rest.
    for kept in file_edges_tx(conn, slug)? {
        let cited = edges
            .iter()
            .any(|e| e.target_kind == KIND_FILE && e.target_id == kept.target_id);
        if !cited && path_under_any_dir(&kept.target_id, &page.dir_refs) {
            edges.push(kept);
        }
    }
    merge_source_tx(conn, KIND_WIKI, slug, edges.clone())?;
    for path in verified {
        let edge = edges
            .iter()
            .find(|e| {
                e.target_kind == KIND_FILE && e.target_id == *path && e.ref_type == RT_WIKI_FILE
            })
            .cloned()
            .unwrap_or_else(|| PageRefEdge::new(KIND_WIKI, slug, KIND_FILE, path, RT_WIKI_FILE));
        upsert_edge_tx(conn, &pin.stamp(edge))?;
    }

    let page_ref = page_ref(slug);
    let written = Written {
        page: page_ref.clone(),
        outbound: outbound_tx(conn, slug)?,
        snapshot: pin.snapshot_id.map(|id| format!("snap:{id}")),
    };
    let env = ev
        .typed::<KnowledgePageWritten>(&KnowledgePageWrittenV1 {
            page: page_ref.clone(),
            outbound: written.outbound.clone(),
            snapshot: written.snapshot.clone(),
        })
        .with_anchors(Anchors {
            snapshot_id: pin.snapshot_id,
            ..anchors.clone()
        })
        .with_subject([page_ref]);
    Ok((written, env))
}

/// Delete a page's row and edges, answering its `knowledge.page.deleted`
/// for the caller to log; `None` when there was no page.
pub fn delete_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    anchors: Anchors,
    slug: &str,
) -> Result<Option<Envelope>, DomainError> {
    let existed = oxplow_db::wiki_page_store::delete_tx(conn, slug)?;
    replace_source_tx(conn, KIND_WIKI, slug, Vec::new())?;
    Ok(existed.then(|| {
        let page = page_ref(slug);
        ev.typed::<KnowledgePageDeleted>(&KnowledgePageDeletedV1 { page: page.clone() })
            .with_anchors(anchors)
            .with_subject([page])
    }))
}

/// The hash a page row keeps of the body it was written from.
pub fn body_hash(body: &str) -> String {
    crate::blob_store::BlobStore::hash(body.as_bytes())
}

fn file_edges_tx(conn: &rusqlite::Connection, slug: &str) -> Result<Vec<PageRefEdge>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT target_id, ref_type, source_extra, local_snapshot_id, closest_vcs_rev,
                    vcs_rev_exact
             FROM page_ref WHERE source_kind = ?1 AND source_id = ?2 AND target_kind = ?3",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![KIND_WIKI, slug, KIND_FILE], |r| {
            let mut edge = PageRefEdge::new(
                KIND_WIKI,
                slug,
                KIND_FILE,
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
            );
            edge.source_extra = r.get(2)?;
            edge.local_snapshot_id = r.get(3)?;
            edge.closest_vcs_rev = r.get(4)?;
            edge.vcs_rev_exact = r.get::<_, i64>(5)? != 0;
            Ok(edge)
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(sql)?;
    Ok(rows)
}

fn outbound_tx(conn: &rusqlite::Connection, slug: &str) -> Result<Vec<String>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT target_kind || ':' || target_id FROM page_ref
             WHERE source_kind = ?1 AND source_id = ?2 ORDER BY 1",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![KIND_WIKI, slug], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(sql)?;
    Ok(rows)
}

/// `body` with its first `# ` heading set to `title` (added when there
/// is none).
fn with_title(body: &str, title: &str) -> String {
    let mut lines: Vec<&str> = body.lines().collect();
    let heading = format!("# {title}");
    match lines.iter().position(|l| l.trim_start().starts_with("# ")) {
        Some(i) => lines[i] = &heading,
        None => {
            return format!("{heading}\n\n{body}");
        }
    }
    let mut out = lines.join("\n");
    if body.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// oxplow's wiki as a store: a page is `.oxplow/wiki/<slug>.md`. It keeps
/// the file; the record is core's ([`write_page_tx`], run by the
/// commands). A hand edit of the file converges through the watcher.
pub struct WikiKnowledge {
    /// The id its declaration gives (`oxplow`).
    pub id: String,
    pub project_dir: PathBuf,
}

#[async_trait::async_trait]
impl KnowledgeProvider for WikiKnowledge {
    fn id(&self) -> &str {
        &self.id
    }

    async fn write_page(
        &self,
        _call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Option<Kept>, CommandError> {
        write_file(&page_path(&self.project_dir, &draft.slug), &draft.body)?;
        Ok(Some(Kept {
            body: draft.body.clone(),
            events: Vec::new(),
        }))
    }

    async fn delete_page(
        &self,
        _call: &PageCall<'_>,
        slug: &str,
    ) -> Result<Option<Vec<Envelope>>, CommandError> {
        let path = page_path(&self.project_dir, slug);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(CommandError::Failed {
                message: format!("couldn't delete {}: {e}", path.display()),
            }),
            _ => Ok(Some(Vec::new())),
        }
    }

    async fn link(
        &self,
        _call: &PageCall<'_>,
        slug: &str,
        target: &str,
    ) -> Result<Option<Kept>, CommandError> {
        let path = page_path(&self.project_dir, slug);
        let current = std::fs::read_to_string(&path)
            .map_err(|_| invalid("/page", format!("no page `{slug}`")))?;
        let body = strip_body_version_literals(&with_related(&current, &format!("- [[{target}]]")));
        write_file(&path, &body)?;
        Ok(Some(Kept {
            body,
            events: Vec::new(),
        }))
    }
}

/// None as a knowledge store: a sink, as the work list's none is. A write
/// lands nowhere and succeeds (`{ tracked: false }`), a delete and a link
/// too, and nothing is recorded, so nothing that writes pages is refused
/// while no store is active.
pub struct NoneKnowledge;

#[async_trait::async_trait]
impl KnowledgeProvider for NoneKnowledge {
    fn id(&self) -> &str {
        oxplow_domain::capability::NONE
    }

    fn sink(&self) -> bool {
        true
    }

    async fn write_page(
        &self,
        _call: &PageCall<'_>,
        _draft: &PageDraft,
    ) -> Result<Option<Kept>, CommandError> {
        Ok(None)
    }

    async fn delete_page(
        &self,
        _call: &PageCall<'_>,
        _slug: &str,
    ) -> Result<Option<Vec<Envelope>>, CommandError> {
        Ok(None)
    }

    async fn link(
        &self,
        _call: &PageCall<'_>,
        _slug: &str,
        _target: &str,
    ) -> Result<Option<Kept>, CommandError> {
        Ok(None)
    }
}

/// Whether oxplow's wiki is the project's knowledge store now: its file
/// watch and its repair (`oxplow.knowledge.resync`) run only then, since
/// the record is the active store's pages. Decided by which ids the wiki
/// was registered under, never by an id's name.
#[derive(Debug, Default)]
pub struct WikiFollow {
    serves: std::sync::RwLock<std::collections::BTreeSet<String>>,
    live: std::sync::atomic::AtomicBool,
}

impl WikiFollow {
    fn set_serves(&self, ids: impl IntoIterator<Item = String>) {
        *self.serves.write().unwrap_or_else(|e| e.into_inner()) = ids.into_iter().collect();
    }

    /// Follow the active store `id`; whether the wiki is it.
    pub fn follow(&self, id: &str) -> bool {
        let live = self
            .serves
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id);
        self.live.store(live, std::sync::atomic::Ordering::SeqCst);
        live
    }

    pub fn live(&self) -> bool {
        self.live.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// A test's wiki, active under `id`.
    #[cfg(test)]
    pub(crate) fn set_serves_for_test(&self, id: &str) {
        self.set_serves([id.to_string()]);
        self.follow(id);
    }
}

/// Empty the record — every page row, its edges and touches — when the
/// store it held pages of is no longer the active one. Nothing in any
/// store is touched.
pub async fn clear_record(db: &oxplow_db::Database) -> Result<(), DomainError> {
    db.transaction(|tx| {
        tx.execute_batch(
            "DELETE FROM page_ref WHERE source_kind = 'wiki';
             DELETE FROM wiki_page_thread_update;
             DELETE FROM wiki_page;",
        )
        .map_err(sql)
    })
    .await
}

/// On `capability.switched` for knowledge: the record becomes the newly
/// active store's pages — emptied, then, for the wiki, restated from its
/// files.
pub struct KnowledgeSwitch {
    pub stores: Arc<oxplow_domain::knowledge::KnowledgeRegistry>,
    pub wiki: Arc<WikiFollow>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub project_dir: PathBuf,
    pub pages: Arc<oxplow_db::SqliteWikiPageStore>,
}

impl KnowledgeSwitch {
    pub const NAME: &'static str = "knowledge.switch";
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for KnowledgeSwitch {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type
            == <oxplow_domain::events::schema::CapabilitySwitched as oxplow_domain::EventType>::TYPE
    }

    async fn handle(&self, event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        if event.envelope.payload["capability"] != CAPABILITY {
            return Ok(());
        }
        clear_record(&self.db).await?;
        if self.wiki.follow(&self.stores.active_id()) {
            crate::wiki_pages::scan_and_sync_all(
                &self.db,
                &self.vocabulary,
                &self.project_dir,
                &self.pages,
            )
            .await?;
        }
        Ok(())
    }
}

/// Register the knowledge stores: core's none (the sink, always there) and
/// the wiki under each id `declared` gives it, which `wiki` follows.
pub fn register_built_ins(
    registry: &oxplow_domain::knowledge::KnowledgeRegistry,
    declared: &[crate::capabilities::Implementation],
    project_dir: &Path,
    wiki: &WikiFollow,
) {
    let mut all: Vec<Arc<dyn KnowledgeProvider>> = vec![Arc::new(NoneKnowledge)];
    all.extend(
        declared
            .iter()
            .filter(|i| i.capability == CAPABILITY)
            .filter(|i| matches!(i.source, crate::capabilities::Source::BuiltIn(BUILT_IN)))
            .map(|i| -> Arc<dyn KnowledgeProvider> {
                Arc::new(WikiKnowledge {
                    id: i.id.clone(),
                    project_dir: project_dir.to_path_buf(),
                })
            }),
    );
    wiki.set_serves(all.iter().filter(|s| !s.sink()).map(|s| s.id().to_string()));
    registry.set_declared(all);
    wiki.follow(&registry.active_id());
}

/// How current each of `page`'s pinned file refs is (`v_knowledge_ref`:
/// the one definition of staleness), whichever store keeps it.
pub async fn freshness(
    db: &oxplow_db::Database,
    page: &str,
) -> Result<Vec<RefFreshness>, DomainError> {
    let page = page.to_string();
    db.read(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT path, pinned_snapshot_id, latest_snapshot_id, stale,
                        pinned_vcs_rev, pinned_vcs_rev_exact
                 FROM v_knowledge_ref WHERE page = ?1 ORDER BY path",
            )
            .map_err(sql)?;
        let rows = stmt
            .query_map(params![page], |r| {
                let target: String = r.get(0)?;
                Ok(RefFreshness {
                    target: format!("{KIND_FILE}:{target}"),
                    pinned_snapshot: r.get(1)?,
                    pinned_revision: r.get(4)?,
                    pinned_revision_exact: r.get::<_, i64>(5)? != 0,
                    latest_snapshot: r.get(2)?,
                    stale: r.get::<_, i64>(3)? != 0,
                })
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(sql)?;
        Ok(rows)
    })
    .await
}

/// Marks a page touched by the thread whose command wrote it (the rail's
/// "Finished" list; `wiki_page_thread_update`), from
/// `knowledge.page.written`. A hand edit (no thread) marks nothing.
pub struct WikiAttribution;

impl WikiAttribution {
    pub const NAME: &'static str = "wiki.attribution";
}

impl crate::event_pump::EventConsumer for WikiAttribution {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <KnowledgePageWritten as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let Some(thread) = event.envelope.anchors.thread_id else {
            return Ok(());
        };
        let written: KnowledgePageWrittenV1 =
            serde_json::from_value(event.envelope.payload.clone()).map_err(|e| {
                DomainError::Invalid(format!("knowledge.page.written payload: {e}"))
            })?;
        let Some(slug) = written.page.strip_prefix("wiki:") else {
            return Ok(());
        };
        // A page deleted since is no longer there to mark.
        if oxplow_db::wiki_page_store::get_tx(conn, slug)?.is_none() {
            return Ok(());
        }
        oxplow_db::wiki_page_thread_updates::touch_tx(conn, thread, slug, event.envelope.at)
    }
}

/// What the commands run against: the active store, and core's record.
#[derive(Clone)]
pub struct KnowledgeTarget {
    pub project_dir: PathBuf,
    pub vcs: Arc<dyn Vcs>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub stores: Arc<oxplow_domain::knowledge::KnowledgeRegistry>,
    pub wiki: Arc<WikiFollow>,
}

fn slug_of(raw: &str) -> Result<&str, CommandError> {
    if valid_slug(raw) {
        Ok(raw)
    } else {
        Err(invalid(
            "/slug",
            format!("`{raw}` isn't a slug (kebab-case: `some-page`)"),
        ))
    }
}

fn domain(e: DomainError) -> CommandError {
    match e {
        DomainError::Invalid(message) => CommandError::Invalid {
            field: None,
            message,
        },
        other => CommandError::from(other),
    }
}

/// Refuse a body whose links don't all resolve, naming each.
/// Refuse the links `body` adds that don't resolve. A link the page
/// already has (in its file now) isn't the writer's to fix: a cited file
/// or task that has since gone, or a hand edit's bad link, mustn't block
/// verifying, rewriting or linking the page.
fn links_resolve(
    target: &KnowledgeTarget,
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::refs::kind::KindRegistry,
    slug: &str,
    body: &str,
) -> Result<(), CommandError> {
    let graph = target.vcs.revision_graph(&target.project_dir);
    let existing: std::collections::HashSet<String> = recorded_body_tx(conn, slug)?
        .map(|current| {
            oxplow_domain::refs::classify_wikilinks(kinds, &current)
                .into_iter()
                .map(|l| l.raw)
                .collect()
        })
        .unwrap_or_default();
    let mut warnings = check_links_in(
        &LinkWorld {
            conn,
            kinds,
            project_dir: &target.project_dir,
            graph: &*graph,
            this_page: Some(slug),
        },
        body,
    );
    warnings.retain(|w| !existing.contains(&w.target));
    if warnings.is_empty() {
        return Ok(());
    }
    Err(invalid(
        "/body",
        format!(
            "links that don't resolve: {}",
            warnings
                .iter()
                .map(|w| format!("[[{}]] ({})", w.target, w.reason))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    ))
}

/// The body core recorded for `slug`, if it records the page.
fn recorded_body_tx(
    conn: &rusqlite::Connection,
    slug: &str,
) -> Result<Option<String>, CommandError> {
    conn.query_row("SELECT body FROM wiki_page WHERE slug = ?1", [slug], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| CommandError::Failed {
        message: e.to_string(),
    })
}

/// `body` with `line` added at the end of its `## Related` section (before
/// whatever section follows), or a new `## Related` at the end.
fn with_related(body: &str, line: &str) -> String {
    let mut lines: Vec<&str> = body.lines().collect();
    let Some(heading) = lines.iter().position(|l| l.trim_end() == "## Related") else {
        return format!("{}\n\n## Related\n\n{line}\n", body.trim_end());
    };
    let end = lines[heading + 1..]
        .iter()
        .position(|l| l.starts_with("# ") || l.starts_with("## "))
        .map_or(lines.len(), |i| heading + 1 + i);
    match (heading + 1..end)
        .rev()
        .find(|&i| !lines[i].trim().is_empty())
    {
        Some(last) => lines.insert(last + 1, line),
        None => lines
            .splice(heading + 1..heading + 1, ["", line])
            .for_each(drop),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Write `body` to the page's file (unless it already holds it), inside
/// the run: the file is the page, so a write that fails fails the run and
/// the transaction records nothing. A temp file renamed into place means
/// the file is never half-written; were the commit itself to fail after
/// it, the file is ahead of the row and the watcher converges them.
fn write_file(path: &Path, body: &str) -> Result<(), CommandError> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(body) {
        return Ok(());
    }
    let failed = |e: std::io::Error| CommandError::Failed {
        message: format!("couldn't write {}: {e}", path.display()),
    };
    let dir = path.parent().ok_or_else(|| CommandError::Failed {
        message: format!("{} has no directory", path.display()),
    })?;
    std::fs::create_dir_all(dir).map_err(failed)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&temp, body)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            failed(e)
        })
}

/// Write a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WritePageInput {
    /// Kebab-case (`vcs-capability`); the page is `.oxplow/wiki/<slug>.md`.
    pub slug: String,
    /// Sets the body's `# ` heading (added when it has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The whole page, markdown. Every `[[link]]` must resolve.
    pub body: String,
    /// Files you re-read against this body: their pins move to now. A
    /// file under a `[[dir:…]]` the body cites counts.
    #[serde(default)]
    pub verified_refs: Vec<String>,
    /// Files you took out of the body.
    #[serde(default)]
    pub removed_refs: Vec<String>,
}

/// Delete a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlugInput {
    pub slug: String,
}

/// Add a link to a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkInput {
    /// The page (its slug).
    pub page: String,
    /// What to link to, as a wikilink's inside: `other-page`,
    /// `src/lib.rs`, `dir:src/vcs`, `tsk42`, `git:<sha>`.
    pub target: String,
}

/// The active store; none is one too (the sink).
fn store(target: &KnowledgeTarget) -> Result<Arc<dyn KnowledgeProvider>, CommandError> {
    target.stores.active().ok_or_else(|| CommandError::Failed {
        message: format!(
            "the knowledge store `{}` isn't running",
            target.stores.active_id()
        ),
    })
}

/// Run `check` against core's record, in a read.
async fn check<F>(target: &KnowledgeTarget, check: F) -> Result<(), CommandError>
where
    F: FnOnce(
            &rusqlite::Connection,
            &oxplow_domain::vocabulary::Vocabulary,
        ) -> Result<(), CommandError>
        + Send
        + 'static,
{
    let vocabulary = target.vocabulary.current();
    target
        .db
        .read(move |conn| Ok(check(conn, &vocabulary)))
        .await
        .map_err(CommandError::from)?
}

/// Record what a store kept of `slug`: its row, links, pins and
/// `knowledge.page.written`, returned for the bus to log after the run.
async fn record(
    target: &KnowledgeTarget,
    actor: &oxplow_domain::Actor,
    slug: &str,
    kept: Kept,
    verified: Vec<String>,
    removed: Vec<String>,
) -> Result<HandlerOutput, CommandError> {
    let (slug, project_dir, vocabulary) = (
        slug.to_string(),
        target.project_dir.clone(),
        target.vocabulary.clone(),
    );
    let (source, anchors) = (actor.source(), actor.anchors());
    let body = strip_body_version_literals(&kept.body);
    let (written, env) = target
        .db
        .transaction(move |tx| {
            let vocabulary = vocabulary.current();
            let ev = EventCtx {
                vocabulary: &vocabulary,
                source: source.clone(),
                cause: None,
            };
            write_page_tx(
                tx,
                &ev,
                &project_dir,
                &PageWrite {
                    slug: &slug,
                    body: &body,
                    verified: &verified,
                    removed: &removed,
                    updated_at: Timestamp::now(),
                    anchors: anchors.clone(),
                },
            )
        })
        .await
        .map_err(domain)?;
    let mut events = kept.events;
    events.push(env);
    Ok(output(
        serde_json::to_value(&written).expect("Written serializes"),
        events,
    ))
}

fn output(result: serde_json::Value, events: Vec<Envelope>) -> HandlerOutput {
    HandlerOutput {
        result,
        inverse: None,
        events,
        after_commit: None,
        unchanged: false,
    }
}

/// What a write to a store that keeps nothing answers.
fn untracked(slug: &str) -> HandlerOutput {
    output(
        json!({ "page": page_ref(slug), "tracked": false }),
        Vec::new(),
    )
}

fn call(invocation: &Invocation) -> PageCall<'_> {
    PageCall {
        actor: &invocation.actor,
        idempotency_key: invocation.idempotency_key.clone(),
    }
}

/// The page commands: each is checked against core's record (the slug,
/// every link it adds, the refs it verifies or removes), then dispatched
/// to the active store, and what the store kept is recorded the same way
/// for every store. `oxplow.knowledge.resync` is the wiki's: it restates a
/// page from its file.
pub fn ops(target: KnowledgeTarget) -> Vec<Op> {
    let t = target.clone();
    let write = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: WritePageInput = parse(input)?;
            let slug = slug_of(&input.slug)?.to_string();
            let mut body = strip_body_version_literals(&input.body);
            if let Some(title) = &input.title {
                body = with_title(&body, title);
            }
            {
                let (t2, slug, body) = (t.clone(), slug.clone(), body.clone());
                let (verified, removed) = (input.verified_refs.clone(), input.removed_refs.clone());
                check(&t, move |conn, vocabulary| {
                    links_resolve(&t2, conn, &vocabulary.kinds, &slug, &body)?;
                    refs_borne_out(&body, &verified, &removed)
                })
                .await?;
            }
            let draft = PageDraft {
                slug: slug.clone(),
                title: input.title.clone(),
                body,
                verified_refs: input.verified_refs.clone(),
                removed_refs: input.removed_refs.clone(),
            };
            match store(&t)?.write_page(&call(&invocation), &draft).await? {
                None => Ok(untracked(&slug)),
                Some(kept) => {
                    record(
                        &t,
                        &invocation.actor,
                        &slug,
                        kept,
                        input.verified_refs,
                        input.removed_refs,
                    )
                    .await
                }
            }
        }) as crate::commands::ExternalFuture
    }));

    let t = target.clone();
    let delete = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: SlugInput = parse(input)?;
            let slug = slug_of(&input.slug)?.to_string();
            let store = store(&t)?;
            {
                let slug = slug.clone();
                let sink = store.sink();
                check(&t, move |conn, _| match recorded_body_tx(conn, &slug)? {
                    None if !sink => Err(invalid("/slug", format!("no page `{slug}`"))),
                    _ => Ok(()),
                })
                .await?;
            }
            let Some(mut events) = store.delete_page(&call(&invocation), &slug).await? else {
                return Ok(untracked(&slug));
            };
            let (vocabulary, source, anchors) = (
                t.vocabulary.clone(),
                invocation.actor.source(),
                invocation.actor.anchors(),
            );
            let deleted = {
                let slug = slug.clone();
                t.db.transaction(move |tx| {
                    let vocabulary = vocabulary.current();
                    let ev = EventCtx {
                        vocabulary: &vocabulary,
                        source: source.clone(),
                        cause: None,
                    };
                    delete_page_tx(tx, &ev, anchors.clone(), &slug)
                })
                .await
                .map_err(domain)?
            };
            events.extend(deleted);
            Ok(output(json!({ "page": page_ref(&slug) }), events))
        }) as crate::commands::ExternalFuture
    }));

    let t = target.clone();
    let link = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: LinkInput = parse(input)?;
            let slug = valid_slug(&input.page)
                .then_some(input.page.clone())
                .ok_or_else(|| invalid("/page", format!("`{}` isn't a slug", input.page)))?;
            let store = store(&t)?;
            {
                let (t2, slug, target) = (t.clone(), slug.clone(), input.target.clone());
                let sink = store.sink();
                check(&t, move |conn, vocabulary| {
                    let Some(current) = recorded_body_tx(conn, &slug)? else {
                        return if sink {
                            Ok(())
                        } else {
                            Err(invalid("/page", format!("no page `{slug}`")))
                        };
                    };
                    let line = format!("- [[{target}]]");
                    if current.lines().any(|l| l.trim() == line) {
                        return Err(invalid("/target", format!("`{slug}` already links to it")));
                    }
                    links_resolve(&t2, conn, &vocabulary.kinds, &slug, &line)
                })
                .await?;
            }
            match store.link(&call(&invocation), &slug, &input.target).await? {
                None => Ok(untracked(&slug)),
                Some(kept) => {
                    record(&t, &invocation.actor, &slug, kept, Vec::new(), Vec::new()).await
                }
            }
        }) as crate::commands::ExternalFuture
    }));

    let t = target;
    let resync = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: SlugInput = parse(input)?;
        let slug = slug_of(&input.slug)?;
        if !t.wiki.live() {
            return Err(invalid(
                "/slug",
                "oxplow's wiki isn't the project's knowledge store: there is no file to restate from",
            ));
        }
        let (result, events) = match std::fs::read_to_string(page_path(&t.project_dir, slug)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let env = delete_page_tx(ctx.conn, &ctx.events, ctx.actor.anchors(), slug)
                    .map_err(domain)?;
                (
                    json!({ "page": page_ref(slug), "deleted": true }),
                    env.into_iter().collect(),
                )
            }
            Err(e) => {
                return Err(CommandError::Failed {
                    message: format!("read the page's file: {e}"),
                })
            }
            Ok(raw) => {
                let body = strip_body_version_literals(&raw);
                let (written, env) = write_page_tx(
                    ctx.conn,
                    &ctx.events,
                    &t.project_dir,
                    &PageWrite {
                        slug,
                        body: &body,
                        verified: &[],
                        removed: &[],
                        updated_at: Timestamp::now(),
                        anchors: ctx.actor.anchors(),
                    },
                )
                .map_err(domain)?;
                (
                    serde_json::to_value(&written).expect("Written serializes"),
                    vec![env],
                )
            }
        };
        Ok(output(result, events))
    }));

    vec![
        Op::new(
            "knowledge.write",
            "write_page",
            schema::<WritePageInput>(),
            false,
            write,
        ),
        Op::new(
            "knowledge.write",
            "delete_page",
            schema::<SlugInput>(),
            false,
            delete,
        )
        .confirm_at_least(Confirm::Destructive),
        Op::new(
            "knowledge.write",
            "link",
            schema::<LinkInput>(),
            false,
            link,
        ),
        Op::new(
            "knowledge.write",
            "resync",
            schema::<SlugInput>(),
            false,
            resync,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    fn dir(fx: &crate::test_fixtures::EffortFixture) -> PathBuf {
        fx.svc.layout.project_dir.clone()
    }

    async fn run(
        fx: &crate::test_fixtures::EffortFixture,
        name: &str,
        input: serde_json::Value,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(&Actor::Human, name, input, true).await
    }

    async fn events_of(
        fx: &crate::test_fixtures::EffortFixture,
        event_type: &str,
    ) -> Vec<oxplow_domain::StoredEvent> {
        fx.svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == event_type)
            .collect()
    }

    /// P5.C3: one run writes the row, the file and the event.
    #[tokio::test]
    async fn one_run_writes_the_row_the_file_and_the_event() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/lib.rs"), "fn x() {}\n").unwrap();
        let body = format!("# VCS notes\n\nSee [[src/lib.rs]] and [[{}]].\n", fx.task);
        let out = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "vcs-notes", "body": body }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["page"], "wiki:vcs-notes");
        let outbound: Vec<&str> = out.result["outbound"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(outbound.contains(&"file:src/lib.rs"), "{outbound:?}");
        let page = fx
            .svc
            .wiki_page_store
            .get("vcs-notes")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.title, "VCS notes");
        assert_eq!(
            std::fs::read_to_string(page_path(&dir(&fx), "vcs-notes")).unwrap(),
            body
        );
        let written = events_of(&fx, "knowledge.page.written").await;
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].envelope.cause, out.event_id);
        assert_eq!(
            written[0].envelope.subject,
            vec!["wiki:vcs-notes".to_string()]
        );

        // The file the command wrote syncs to a no-op, not a second write.
        assert!(!crate::wiki_pages::sync_page(
            &fx.svc.db,
            &fx.svc.vocabulary,
            &dir(&fx),
            "vcs-notes"
        )
        .await
        .unwrap());
        assert_eq!(events_of(&fx, "knowledge.page.written").await.len(), 1);
    }

    /// The file is the page: a file that can't be written fails the run,
    /// and nothing — row, edges, event — is recorded (tsk562).
    #[tokio::test]
    async fn a_file_that_cant_be_written_fails_the_run_and_records_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        // A directory where the page's file goes.
        std::fs::create_dir_all(page_path(&dir(&fx), "blocked")).unwrap();
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "blocked", "body": "# Blocked\n" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("blocked.md"), "{err}");
        assert!(fx
            .svc
            .wiki_page_store
            .get("blocked")
            .await
            .unwrap()
            .is_none());
        assert!(events_of(&fx, "knowledge.page.written").await.is_empty());
    }

    /// tsk572: one slug rule — a wiki file whose name isn't a slug is not
    /// a page: syncing it records nothing, and a row it left is removed.
    #[tokio::test]
    async fn a_file_whose_name_isnt_a_slug_is_not_a_page() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let wiki = dir(&fx).join(".oxplow/wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        std::fs::write(wiki.join("Bad_Name.md"), "# Bad\n").unwrap();
        crate::wiki_pages::scan_and_sync_all(
            &fx.svc.db,
            &fx.svc.vocabulary,
            &dir(&fx),
            &fx.svc.wiki_page_store,
        )
        .await
        .unwrap();
        assert!(fx
            .svc
            .wiki_page_store
            .get("Bad_Name")
            .await
            .unwrap()
            .is_none());
        assert!(events_of(&fx, "knowledge.page.written").await.is_empty());
    }

    /// tsk572: `oxplow.knowledge.link` adds under `## Related` — even when a
    /// section follows it — and the body it writes has no `@version`.
    #[tokio::test]
    async fn link_adds_under_related_and_strips_versions() {
        let fx = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/lib.rs"), "x").unwrap();
        for slug in ["b", "c"] {
            run(&fx, WRITE_PAGE, json!({ "slug": slug, "body": "# P\n" }))
                .await
                .unwrap();
        }
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "a", "body": "# A\n\n## Related\n\n- [[b]]\n\n## Notes\n\nmore\n" }),
        )
        .await
        .unwrap();
        // A hand edit leaves a version literal behind.
        let path = page_path(&dir(&fx), "a");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("more", "see [[src/lib.rs@abc1234]]");
        std::fs::write(&path, edited).unwrap();

        run(&fx, LINK, json!({ "page": "a", "target": "c" }))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# A\n\n## Related\n\n- [[b]]\n- [[c]]\n\n## Notes\n\nsee [[src/lib.rs]]\n"
        );
    }

    /// A dangling link is refused, naming it, and nothing is written.
    #[tokio::test]
    async fn a_dangling_link_is_refused_naming_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "notes", "body": "# N\n\n[[no-such-page]] and [[src/gone.rs]] and [[tsk9999]]\n" }),
        )
        .await
        .unwrap_err();
        let message = err.to_string();
        for target in ["no-such-page", "src/gone.rs", "tsk9999"] {
            assert!(message.contains(target), "{message}");
        }
        assert!(fx.svc.wiki_page_store.get("notes").await.unwrap().is_none());
        assert!(!page_path(&dir(&fx), "notes").exists());
        assert!(events_of(&fx, "knowledge.page.written").await.is_empty());

        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "Not A Slug", "body": "x" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("slug"), "{err}");
        // A page may link to itself.
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "self", "body": "# S\n\nsee [[self]]\n" }),
        )
        .await
        .unwrap();
    }

    /// Only links the page doesn't already have are refused (tsk564): once
    /// a cited file is gone, the page can still be verified, rewritten and
    /// linked — a new dangling link is still refused.
    #[tokio::test]
    async fn a_link_that_went_dangling_doesnt_block_the_page() {
        let fx = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/a.rs"), "a").unwrap();
        std::fs::write(dir(&fx).join("src/b.rs"), "b").unwrap();
        let body = "# P\n\nsee [[src/a.rs]] and [[src/b.rs]]\n";
        run(&fx, WRITE_PAGE, json!({ "slug": "p", "body": body }))
            .await
            .unwrap();
        run(&fx, WRITE_PAGE, json!({ "slug": "q", "body": "# Q\n" }))
            .await
            .unwrap();
        std::fs::remove_file(dir(&fx).join("src/a.rs")).unwrap();

        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "p", "body": body, "verified_refs": ["src/b.rs"] }),
        )
        .await
        .unwrap();
        run(&fx, LINK, json!({ "page": "p", "target": "q" }))
            .await
            .unwrap();
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "p", "body": format!("{body}and [[src/c.rs]]\n") }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("src/c.rs"), "{err}");
        assert!(!err.to_string().contains("src/a.rs"), "{err}");
    }

    /// Verified refs must be cited (or under a cited directory, which
    /// makes an edge of their own); removed ones must be gone.
    #[tokio::test]
    async fn verified_and_removed_refs_are_checked_against_the_body() {
        let fx = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(dir(&fx).join("crates/cp/src")).unwrap();
        std::fs::write(dir(&fx).join("crates/cp/src/lib.rs"), "").unwrap();
        std::fs::write(dir(&fx).join("crates/foo.rs"), "").unwrap();
        let body = "# I\n\nsee [[dir:crates/cp]] and [[crates/foo.rs]]\n";
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "intro", "body": body, "removed_refs": ["crates/foo.rs"] }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("crates/foo.rs"), "{err}");
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "intro", "body": body, "verified_refs": ["crates/other/main.rs"] }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("crates/other/main.rs"), "{err}");
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "intro", "body": body, "verified_refs": ["crates/cp/src/lib.rs"] }),
        )
        .await
        .unwrap();
        let backlinks = fx
            .svc
            .page_ref_store
            .list_backlinks("file", "crates/cp/src/lib.rs", None)
            .await
            .unwrap();
        assert!(
            backlinks.iter().any(|e| e.source_id == "intro"),
            "{backlinks:?}"
        );
        // Rewriting the page (the directory still cited) keeps it.
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "intro", "body": format!("{body}\nMore.\n") }),
        )
        .await
        .unwrap();
        let backlinks = fx
            .svc
            .page_ref_store
            .list_backlinks("file", "crates/cp/src/lib.rs", None)
            .await
            .unwrap();
        assert!(
            backlinks.iter().any(|e| e.source_id == "intro"),
            "{backlinks:?}"
        );
    }

    /// A hand edit converges through the same core, as `system:wiki_watch`
    /// — dangling links and all.
    #[tokio::test]
    async fn a_hand_edit_converges_as_the_watcher() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let path = page_path(&dir(&fx), "hand");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# Hand\n\nsee [[not-yet-written]]\n").unwrap();
        assert!(
            crate::wiki_pages::sync_page(&fx.svc.db, &fx.svc.vocabulary, &dir(&fx), "hand")
                .await
                .unwrap()
        );
        assert_eq!(
            fx.svc
                .wiki_page_store
                .get("hand")
                .await
                .unwrap()
                .unwrap()
                .title,
            "Hand"
        );
        let written = events_of(&fx, "knowledge.page.written").await;
        assert_eq!(written[0].envelope.source, "system:wiki_watch");
        std::fs::remove_file(&path).unwrap();
        assert!(
            crate::wiki_pages::sync_page(&fx.svc.db, &fx.svc.vocabulary, &dir(&fx), "hand")
                .await
                .unwrap()
        );
        assert!(fx.svc.wiki_page_store.get("hand").await.unwrap().is_none());
        assert_eq!(events_of(&fx, "knowledge.page.deleted").await.len(), 1);
    }

    async fn body_of(fx: &crate::test_fixtures::EffortFixture, page: &str) -> Option<String> {
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT body FROM v_knowledge_body WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(page.into())],
                None,
            )
            .await
            .unwrap();
        out.rows.first().map(|r| match &r[0] {
            oxplow_db::SqlCell::Text(t) => t.clone(),
            other => panic!("{other:?}"),
        })
    }

    /// P6.E2: a page's body is in its row (the UI reads `v_knowledge_body`:
    /// `.oxplow/` is ignored, so the file isn't readable through the
    /// workspace), written with it by the command and by the watcher.
    #[tokio::test]
    async fn the_body_reads_back_from_v_knowledge_body() {
        let fx = crate::test_fixtures::services_with_effort().await;
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "notes", "body": "# Notes\n\nFirst.\n" }),
        )
        .await
        .unwrap();
        assert_eq!(
            body_of(&fx, "wiki:notes").await.as_deref(),
            Some("# Notes\n\nFirst.\n")
        );

        std::fs::write(
            page_path(&dir(&fx), "notes"),
            "# Notes\n\nEdited by hand.\n",
        )
        .unwrap();
        assert!(
            crate::wiki_pages::sync_page(&fx.svc.db, &fx.svc.vocabulary, &dir(&fx), "notes")
                .await
                .unwrap()
        );
        assert_eq!(
            body_of(&fx, "wiki:notes").await.as_deref(),
            Some("# Notes\n\nEdited by hand.\n")
        );
    }

    async fn stale_ref_count(fx: &crate::test_fixtures::EffortFixture, page: &str) -> i64 {
        let page = page.to_string();
        fx.svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT stale_ref_count FROM v_knowledge_page WHERE ref = ?1",
                    [page],
                    |r| r.get(0),
                )
                .map_err(sql)
            })
            .await
            .unwrap()
    }

    /// A snapshot of the primary stream in which `path` changed.
    async fn snapshot_with(fx: &crate::test_fixtures::EffortFixture, path: &str) -> i64 {
        let path = path.to_string();
        fx.svc
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
                    params![path, id],
                )
                .map_err(sql)?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    /// Staleness is the primary stream's, like the pin (tsk563): another
    /// stream's newer snapshot of the file neither makes a page stale nor
    /// keeps a verified one stale, and every reader agrees.
    #[tokio::test]
    async fn another_streams_snapshot_doesnt_make_a_page_stale() {
        let fx = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/lib.rs"), "v1").unwrap();
        snapshot_with(&fx, "src/lib.rs").await;
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "lib", "body": "# Lib\n\nsee [[src/lib.rs]]\n" }),
        )
        .await
        .unwrap();
        fx.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                          worktree_path, created_at, updated_at)
                       VALUES (99, 'worktree', 'w', 'w', 'r', 'r', '/w',
                               '2026-09-30T00:00:00.000000Z', '2026-09-30T00:00:00.000000Z');
                     INSERT INTO snapshot (id, stream_id, created_at)
                       VALUES (9999, 99, '2026-09-30T00:00:00.000000Z');
                     INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes,
                                                captured_at, storage, snapshot_id)
                       VALUES (99, 'src/lib.rs', 'h2', 1, '2026-09-30T00:00:00.000000Z',
                               'oxplow', 9999);",
                )
                .map_err(sql)
            })
            .await
            .unwrap();
        assert_eq!(stale_ref_count(&fx, "wiki:lib").await, 0);
        let fresh = freshness(&fx.svc.db, "wiki:lib").await.unwrap();
        assert_eq!(fresh.len(), 1);
        assert!(!fresh[0].stale, "{fresh:?}");
        assert_ne!(fresh[0].latest_snapshot, Some(9999));
    }

    /// P5.C4: `v_knowledge_page.stale_ref_count` rises when a pinned
    /// file drifts and a snapshot lands, and falls when it is verified.
    #[tokio::test]
    async fn stale_ref_count_rises_when_a_pinned_file_drifts() {
        let fx = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/lib.rs"), "v1").unwrap();
        snapshot_with(&fx, "src/lib.rs").await;
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "lib", "body": "# Lib\n\nsee [[src/lib.rs]]\n" }),
        )
        .await
        .unwrap();
        assert_eq!(stale_ref_count(&fx, "wiki:lib").await, 0);

        snapshot_with(&fx, "src/lib.rs").await;
        assert_eq!(stale_ref_count(&fx, "wiki:lib").await, 1);

        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "lib", "body": "# Lib\n\nsee [[src/lib.rs]]\n", "verified_refs": ["src/lib.rs"] }),
        )
        .await
        .unwrap();
        assert_eq!(stale_ref_count(&fx, "wiki:lib").await, 0);
        let outbound: String = fx
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT outbound_refs FROM v_knowledge_page WHERE ref = 'wiki:lib'",
                    [],
                    |r| r.get(0),
                )
                .map_err(sql)
            })
            .await
            .unwrap();
        assert_eq!(outbound, r#"["file:src/lib.rs"]"#);
        let refs: (String, i64) = fx
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT path, stale FROM v_knowledge_ref WHERE page = 'wiki:lib'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(sql)
            })
            .await
            .unwrap();
        assert_eq!(refs, ("src/lib.rs".to_string(), 0));
    }

    /// A page an agent's command writes is marked touched by its thread.
    #[tokio::test]
    async fn a_written_page_is_attributed_to_the_writing_thread() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        fx.svc
            .commands
            .run(
                &agent,
                WRITE_PAGE,
                json!({ "slug": "arch", "body": "# Arch\n" }),
                false,
            )
            .await
            .unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        let touched = fx
            .svc
            .wiki_page_thread_updates
            .list_for_thread(&fx.thread, 10)
            .await
            .unwrap();
        assert_eq!(
            touched.iter().map(|t| t.slug.as_str()).collect::<Vec<_>>(),
            vec!["arch"]
        );
    }

    /// Delete is confirmed and takes the row, the file and the edges;
    /// link adds under Related and refuses a dangling target.
    /// `id` as the project's knowledge store, its switch applied.
    async fn choose(fx: &crate::test_fixtures::EffortFixture, id: &str) {
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), id.into());
        let config = crate::config_service::read_config(&fx.svc.config);
        fx.svc
            .capabilities
            .publish(&config, &fx.svc.db)
            .await
            .unwrap();
        fx.svc
            .event_pump
            .settle(&[KnowledgeSwitch::NAME], std::time::Duration::from_secs(10))
            .await;
    }

    async fn recorded(fx: &crate::test_fixtures::EffortFixture) -> Vec<String> {
        fx.svc
            .wiki_page_store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.slug)
            .collect()
    }

    /// The record is the active store's pages: choosing another store
    /// empties it (nothing in the wiki's files is touched) and the wiki's
    /// watch and repair stand down; choosing the wiki again restates it
    /// from its files.
    #[tokio::test]
    async fn the_record_follows_the_active_store() {
        let fx = crate::test_fixtures::services_with_effort().await;
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "kept", "body": "# Kept\n" }),
        )
        .await
        .unwrap();
        assert_eq!(recorded(&fx).await, ["kept"]);
        assert!(fx.svc.wiki.live());

        choose(&fx, "none").await;
        assert!(recorded(&fx).await.is_empty());
        assert!(dir(&fx).join(".oxplow/wiki/kept.md").exists());
        assert!(!fx.svc.wiki.live());
        let err = run(&fx, RESYNC, json!({ "slug": "kept" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("isn't the project's knowledge store"),
            "{err}"
        );

        choose(&fx, "oxplow").await;
        assert!(fx.svc.wiki.live());
        assert_eq!(recorded(&fx).await, ["kept"]);
    }

    /// A store that records what it's asked, and keeps the body it's given.
    struct Recording(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl KnowledgeProvider for Recording {
        fn id(&self) -> &str {
            "vault"
        }
        async fn write_page(
            &self,
            call: &PageCall<'_>,
            draft: &PageDraft,
        ) -> Result<Option<Kept>, CommandError> {
            self.0.lock().unwrap().push(format!(
                "write {} {:?} by {}",
                draft.slug,
                draft.verified_refs,
                call.actor.source()
            ));
            Ok(Some(Kept {
                body: draft.body.clone(),
                events: Vec::new(),
            }))
        }
        async fn delete_page(
            &self,
            _: &PageCall<'_>,
            slug: &str,
        ) -> Result<Option<Vec<Envelope>>, CommandError> {
            self.0.lock().unwrap().push(format!("delete {slug}"));
            Ok(Some(Vec::new()))
        }
        async fn link(
            &self,
            _: &PageCall<'_>,
            slug: &str,
            target: &str,
        ) -> Result<Option<Kept>, CommandError> {
            self.0.lock().unwrap().push(format!("link {slug} {target}"));
            Ok(Some(Kept {
                body: format!("# Kept\n\n- [[{target}]]\n"),
                events: Vec::new(),
            }))
        }
    }

    /// The page commands reach whichever store is active, checked first
    /// against core's record, and core records what the store kept: the
    /// wiki's files aren't touched while another store is chosen.
    #[tokio::test]
    async fn the_commands_write_through_the_active_store() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let vault = Arc::new(Recording(Default::default()));
        fx.svc.knowledge.register(vault.clone());
        // Running, as a provider's instance is.
        fx.svc.capabilities.set_external(
            crate::capabilities::Implementation {
                capability: CAPABILITY.into(),
                id: "vault".into(),
                title: "Vault".into(),
                extension: Some("vault".into()),
                source: crate::capabilities::Source::External,
                features: json!({}),
                fields: json!([]),
                id_pattern: None,
                config: json!({}),
            },
            true,
        );
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), "vault".into());
        std::fs::create_dir_all(dir(&fx).join("src")).unwrap();
        std::fs::write(dir(&fx).join("src/lib.rs"), "fn a() {}\n").unwrap();

        let out = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "kept", "body": "# Kept\n\nSee [[src/lib.rs]].\n",
                    "verified_refs": ["src/lib.rs"] }),
        )
        .await
        .unwrap();
        assert_eq!(out.result["page"], "wiki:kept");
        assert!(!dir(&fx).join(".oxplow/wiki/kept.md").exists());
        assert_eq!(
            fx.svc
                .wiki_page_store
                .get("kept")
                .await
                .unwrap()
                .map(|p| p.title),
            Some("Kept".into())
        );
        // A dangling link never reaches the store.
        let err = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "kept", "body": "# Kept\n\n[[no-such-page]]\n" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no-such-page"), "{err}");
        run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "other", "body": "# Other\n" }),
        )
        .await
        .unwrap();
        run(&fx, LINK, json!({ "page": "kept", "target": "other" }))
            .await
            .unwrap();
        run(&fx, DELETE_PAGE, json!({ "slug": "other" }))
            .await
            .unwrap();
        assert_eq!(
            *vault.0.lock().unwrap(),
            [
                r#"write kept ["src/lib.rs"] by human"#.to_string(),
                "write other [] by human".into(),
                "link kept other".into(),
                "delete other".into(),
            ]
        );
        assert!(fx.svc.wiki_page_store.get("other").await.unwrap().is_none());
    }

    /// Knowledge is a choice: oxplow's wiki (bundled) or none, which is a
    /// sink — a write through it lands nowhere and succeeds.
    #[tokio::test]
    async fn none_is_a_choice_and_a_sink() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT provider, active, choosable, optional FROM v_capability_provider
                  WHERE capability = 'knowledge' ORDER BY provider",
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .rows;
        use oxplow_db::SqlCell::{Int, Text};
        assert_eq!(
            rows,
            vec![
                vec![Text("none".into()), Int(0), Int(1), Int(1)],
                vec![Text("oxplow".into()), Int(1), Int(1), Int(1)],
            ]
        );
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), "none".into());
        let out = run(
            &fx,
            WRITE_PAGE,
            json!({ "slug": "kept-nowhere", "body": "# Kept nowhere\n" }),
        )
        .await
        .unwrap();
        assert_eq!(
            out.result,
            json!({ "page": "wiki:kept-nowhere", "tracked": false })
        );
        assert!(fx
            .svc
            .wiki_page_store
            .get("kept-nowhere")
            .await
            .unwrap()
            .is_none());
        assert!(!dir(&fx).join(".oxplow/wiki/kept-nowhere.md").exists());
        // A delete and a link of a page none keeps succeed too.
        run(&fx, DELETE_PAGE, json!({ "slug": "kept-nowhere" }))
            .await
            .unwrap();
        run(
            &fx,
            LINK,
            json!({ "page": "kept-nowhere", "target": "other" }),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn delete_and_link() {
        let fx = crate::test_fixtures::services_with_effort().await;
        run(&fx, WRITE_PAGE, json!({ "slug": "a", "body": "# A\n" }))
            .await
            .unwrap();
        run(&fx, WRITE_PAGE, json!({ "slug": "b", "body": "# B\n" }))
            .await
            .unwrap();

        let err = run(&fx, LINK, json!({ "page": "a", "target": "nope" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
        let out = run(&fx, LINK, json!({ "page": "a", "target": "b" }))
            .await
            .unwrap();
        assert!(out.result["outbound"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "wiki:b"));
        assert_eq!(
            std::fs::read_to_string(page_path(&dir(&fx), "a")).unwrap(),
            "# A\n\n## Related\n\n- [[b]]\n"
        );

        let err = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE_PAGE, json!({ "slug": "b" }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        run(&fx, DELETE_PAGE, json!({ "slug": "b" })).await.unwrap();
        assert!(fx.svc.wiki_page_store.get("b").await.unwrap().is_none());
        assert!(!page_path(&dir(&fx), "b").exists());
        let err = run(&fx, DELETE_PAGE, json!({ "slug": "b" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no page"), "{err}");
    }
}
