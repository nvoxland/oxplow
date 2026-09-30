//! Knowledge — the wiki — as commands (P5.C3, `.context/knowledge.md`).
//!
//! `knowledge.write_page` is the one way a page is written: in the bus's
//! transaction it validates the slug and every `[[link]]`, restates the
//! `wiki_page` row and its `page_ref` edges (new file refs pinned to the
//! primary stream's latest snapshot) and logs `knowledge.page.written`;
//! once committed it writes `.oxplow/wiki/<slug>.md`. Beside it:
//! `knowledge.delete_page`, `knowledge.link` (add a link to a page) and
//! `knowledge.resync` (restate a page from its file — the repair op).
//! A hand edit of the file converges through the same core
//! ([`crate::wiki_pages::sync_page`], logged as `system:wiki_watch`).

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
use oxplow_domain::vcs::{Revision, Vcs};
use oxplow_domain::{
    Anchors, Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, DomainError, Invokers,
    Lifecycle, Timestamp,
};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::{Command, Handler, HandlerOutput, TxCtx};
use crate::events::{EventBus, OxplowEvent};
use crate::link_check::{check_links_in, LinkWorld};
use crate::wiki_pages::{
    extract_title, parse_refs, path_under_any_dir, strip_body_version_literals, wiki_pages_dir,
};

pub const WRITE_PAGE: &str = "knowledge.write_page";
pub const DELETE_PAGE: &str = "knowledge.delete_page";
pub const LINK: &str = "knowledge.link";
pub const RESYNC: &str = "knowledge.resync";

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

/// Restate a page's row and edges from its body and log
/// `knowledge.page.written`. Edges already stored keep their pins (so
/// unrelated prose edits don't re-stamp them); new file refs and verified
/// ones take the current [`Pin`]. A file verified under a directory the
/// body cites (`[[dir:…]]`) becomes an edge of its own, kept while that
/// directory stays cited.
pub fn write_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    project_dir: &Path,
    write: &PageWrite<'_>,
) -> Result<Written, DomainError> {
    let PageWrite {
        slug,
        body,
        verified,
        removed,
        updated_at,
        ref anchors,
    } = *write;
    let refs = parse_refs(body);
    let still_cited: Vec<&str> = removed
        .iter()
        .filter(|p| refs.file_refs.contains(p))
        .map(String::as_str)
        .collect();
    if !still_cited.is_empty() {
        return Err(DomainError::Invalid(format!(
            "removed_refs still cited by the body: {}",
            still_cited.join(", ")
        )));
    }
    let unreferenced: Vec<&str> = verified
        .iter()
        .filter(|p| !refs.file_refs.contains(p) && !path_under_any_dir(p, &refs.dir_refs))
        .map(String::as_str)
        .collect();
    if !unreferenced.is_empty() {
        return Err(DomainError::Invalid(format!(
            "verified_refs neither cited by the body nor under a cited directory: {}",
            unreferenced.join(", ")
        )));
    }
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
    oxplow_db::wiki_page_store::upsert_tx(conn, &page, &body_hash(body))?;

    let pin = pin_tx(conn)?;
    let mut edges = wiki_edges(slug, body);
    if let Some(id) = pin.snapshot_id {
        stamp_file_versions(
            &mut edges,
            FileRefVersion {
                local_snapshot_id: id,
                closest_git_version: pin.revision.as_deref(),
                git_version_exact: pin.revision.is_some(),
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
    ev.append(conn, &env)?;
    Ok(written)
}

/// Delete a page's row and edges and log `knowledge.page.deleted`;
/// whether there was a page.
pub fn delete_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    anchors: Anchors,
    slug: &str,
) -> Result<bool, DomainError> {
    let existed = oxplow_db::wiki_page_store::delete_tx(conn, slug)?;
    replace_source_tx(conn, KIND_WIKI, slug, Vec::new())?;
    if existed {
        let page = page_ref(slug);
        let env = ev
            .typed::<KnowledgePageDeleted>(&KnowledgePageDeletedV1 { page: page.clone() })
            .with_anchors(anchors)
            .with_subject([page]);
        ev.append(conn, &env)?;
    }
    Ok(existed)
}

/// The hash a page row keeps of the body it was written from.
pub fn body_hash(body: &str) -> String {
    crate::blob_store::BlobStore::hash(body.as_bytes())
}

fn file_edges_tx(conn: &rusqlite::Connection, slug: &str) -> Result<Vec<PageRefEdge>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT target_id, ref_type, source_extra, local_snapshot_id, closest_git_version,
                    git_version_exact
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
            edge.closest_git_version = r.get(4)?;
            edge.git_version_exact = r.get::<_, i64>(5)? != 0;
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

/// What the commands run against.
#[derive(Clone)]
pub struct KnowledgeTarget {
    pub project_dir: PathBuf,
    pub vcs: Arc<dyn Vcs>,
    pub events: EventBus,
}

impl KnowledgeTarget {
    fn changed(&self, slug: &str) -> impl FnOnce() + Send + Sync + 'static {
        let (events, slug) = (self.events.clone(), slug.to_string());
        move || events.emit(OxplowEvent::WikiPagesChanged { slug })
    }
}

fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message: message.into(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
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
fn links_resolve(
    target: &KnowledgeTarget,
    conn: &rusqlite::Connection,
    slug: &str,
    body: &str,
) -> Result<(), CommandError> {
    let graph = target.vcs.revision_graph(&target.project_dir);
    let warnings = check_links_in(
        &LinkWorld {
            conn,
            project_dir: &target.project_dir,
            graph: &*graph,
            this_page: Some(slug),
        },
        body,
    );
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

/// Write `body` to the page's file after the run commits (unless it
/// already holds it) and announce the page.
fn write_file_after(
    target: &KnowledgeTarget,
    slug: &str,
    body: String,
) -> Box<dyn FnOnce() + Send + Sync> {
    let path = page_path(&target.project_dir, slug);
    let announce = target.changed(slug);
    Box::new(move || {
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            return;
        }
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &body));
        if let Err(e) = written {
            tracing::error!(path = %path.display(), error = %e, "couldn't write the wiki page's file");
        }
        announce();
    })
}

fn spec(name: &str, summary: &str, schema: serde_json::Value, confirm: Confirm) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm,
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        // oxplow's own records, like work items: a read-only thread
        // captures what it explored, too.
        effect: CommandEffect::Record,
    }
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

pub fn commands(target: KnowledgeTarget) -> Vec<Command> {
    let t = target.clone();
    let write = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: WritePageInput = parse(input)?;
        let slug = slug_of(&input.slug)?;
        let mut body = strip_body_version_literals(&input.body);
        if let Some(title) = &input.title {
            body = with_title(&body, title);
        }
        links_resolve(&t, ctx.conn, slug, &body)?;
        let written = write_page_tx(
            ctx.conn,
            &ctx.events,
            &t.project_dir,
            &PageWrite {
                slug,
                body: &body,
                verified: &input.verified_refs,
                removed: &input.removed_refs,
                updated_at: Timestamp::now(),
                anchors: ctx.actor.anchors(),
            },
        )
        .map_err(domain)?;
        Ok(HandlerOutput {
            result: serde_json::to_value(&written).expect("Written serializes"),
            inverse: None,
            events: Vec::new(),
            after_commit: Some(write_file_after(&t, slug, body)),
        })
    }));

    let t = target.clone();
    let delete = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: SlugInput = parse(input)?;
        let slug = slug_of(&input.slug)?;
        let path = page_path(&t.project_dir, slug);
        let announce = t.changed(slug);
        let existed =
            delete_page_tx(ctx.conn, &ctx.events, ctx.actor.anchors(), slug).map_err(domain)?;
        if !existed && !path.exists() {
            return Err(invalid("/slug", format!("no page `{slug}`")));
        }
        Ok(HandlerOutput {
            result: json!({ "page": page_ref(slug) }),
            inverse: None,
            events: Vec::new(),
            after_commit: Some(Box::new(move || {
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::error!(path = %path.display(), error = %e, "couldn't delete the wiki page's file");
                    }
                }
                announce();
            })),
        })
    }));

    let t = target.clone();
    let link = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: LinkInput = parse(input)?;
        let slug = valid_slug(&input.page)
            .then_some(input.page.as_str())
            .ok_or_else(|| invalid("/page", format!("`{}` isn't a slug", input.page)))?;
        let current = std::fs::read_to_string(page_path(&t.project_dir, slug))
            .map_err(|_| invalid("/page", format!("no page `{slug}`")))?;
        let line = format!("- [[{}]]", input.target);
        if current.lines().any(|l| l.trim() == line) {
            return Err(invalid("/target", format!("`{slug}` already links to it")));
        }
        let body = match current.find("\n## Related") {
            Some(_) => format!("{}\n{line}\n", current.trim_end()),
            None => format!("{}\n\n## Related\n\n{line}\n", current.trim_end()),
        };
        links_resolve(&t, ctx.conn, slug, &body)?;
        let written = write_page_tx(
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
        Ok(HandlerOutput {
            result: serde_json::to_value(&written).expect("Written serializes"),
            inverse: None,
            events: Vec::new(),
            after_commit: Some(write_file_after(&t, slug, body)),
        })
    }));

    let t = target;
    let resync = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: SlugInput = parse(input)?;
        let slug = slug_of(&input.slug)?;
        let result = match std::fs::read_to_string(page_path(&t.project_dir, slug)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                delete_page_tx(ctx.conn, &ctx.events, ctx.actor.anchors(), slug).map_err(domain)?;
                json!({ "page": page_ref(slug), "deleted": true })
            }
            Err(e) => {
                return Err(CommandError::Failed {
                    message: format!("read the page's file: {e}"),
                })
            }
            Ok(raw) => {
                let body = strip_body_version_literals(&raw);
                let written = write_page_tx(
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
                serde_json::to_value(&written).expect("Written serializes")
            }
        };
        Ok(HandlerOutput {
            result,
            inverse: None,
            events: Vec::new(),
            after_commit: Some(Box::new(t.changed(slug))),
        })
    }));

    vec![
        Command::new(
            spec(
                WRITE_PAGE,
                "Write a knowledge (wiki) page: its body, every [[link]] resolving, and the \
                 files you re-read against it (verified_refs) or took out (removed_refs).",
                serde_json::to_value(schemars::schema_for!(WritePageInput)).expect("schema"),
                Confirm::Never,
            ),
            write,
        )
        .expect("knowledge.write_page registers"),
        Command::new(
            spec(
                DELETE_PAGE,
                "Delete a knowledge (wiki) page and its links.",
                serde_json::to_value(schemars::schema_for!(SlugInput)).expect("schema"),
                Confirm::Destructive,
            ),
            delete,
        )
        .expect("knowledge.delete_page registers"),
        Command::new(
            spec(
                LINK,
                "Link a knowledge page to a page, file, directory, task or commit (added under \
                 its Related heading).",
                serde_json::to_value(schemars::schema_for!(LinkInput)).expect("schema"),
                Confirm::Never,
            ),
            link,
        )
        .expect("knowledge.link registers"),
        Command::new(
            spec(
                RESYNC,
                "Restate a knowledge page from its file on disk (the repair when a row and its \
                 file disagree); a missing file deletes the page.",
                serde_json::to_value(schemars::schema_for!(SlugInput)).expect("schema"),
                Confirm::Never,
            ),
            resync,
        )
        .expect("knowledge.resync registers"),
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
        let fx = crate::test_fixtures::services_with_effort().await;
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
            &fx.svc.event_schemas,
            &dir(&fx),
            "vcs-notes"
        )
        .await
        .unwrap());
        assert_eq!(events_of(&fx, "knowledge.page.written").await.len(), 1);
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
            crate::wiki_pages::sync_page(&fx.svc.db, &fx.svc.event_schemas, &dir(&fx), "hand")
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
            crate::wiki_pages::sync_page(&fx.svc.db, &fx.svc.event_schemas, &dir(&fx), "hand")
                .await
                .unwrap()
        );
        assert!(fx.svc.wiki_page_store.get("hand").await.unwrap().is_none());
        assert_eq!(events_of(&fx, "knowledge.page.deleted").await.len(), 1);
    }

    /// A page an agent's command writes is marked touched by its thread.
    #[tokio::test]
    async fn a_written_page_is_attributed_to_the_writing_thread() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let agent = Actor::Agent {
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
