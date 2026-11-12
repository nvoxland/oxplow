//! Knowledge as commands (`.context/knowledge.md`): pages kept in one or
//! more stores at once — oxwiki (`oxplow_oxwiki`), an extension's — and
//! core's record of them, the same for every store.
//!
//! `oxplow.knowledge.write_page` is the one way a page is written: it
//! resolves the store (named, or the only one enabled), checks the write
//! against core's record (its id, every link it adds, the files it verifies
//! or removes), has the store keep it, and records what the store kept
//! (`oxplow_db::knowledge_record`: the row, its `page_ref` edges with new
//! file refs pinned, `knowledge.page.written`). Beside it:
//! `oxplow.knowledge.delete_page`, `oxplow.knowledge.verify_files` (only the
//! pins move: nothing reaches the store),
//! `oxplow.knowledge.link` (core adds the link under the page's Related
//! heading and writes the page through its store) and
//! `oxplow.knowledge.resync` (restate an oxwiki page from its file — the
//! repair op).

use crate::commands::ops::Op;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::knowledge_record::{
    delete_page_tx, strip_body_version_literals, write_page_tx, CitedRefs, PageWrite,
};
use oxplow_db::EventCtx;
use oxplow_domain::events::schema::{
    AgentToolFinished, AgentToolFinishedV4, KnowledgePageDrifted, KnowledgePageDriftedV1,
    KnowledgePageRead, KnowledgePageReadV1, KnowledgePageWritten, KnowledgePageWrittenV3, ReadVia,
    SnapshotTaken, SnapshotTakenV3,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::knowledge::{
    Kept, KnowledgeProvider, KnowledgeStores, PageCall, PageDraft, RefFreshness, OXWIKI,
};
use oxplow_domain::uri::build::knowledge_uri;
use oxplow_domain::vcs::Vcs;
use oxplow_domain::{CommandError, Confirm, DomainError, Timestamp};
use oxplow_oxwiki::OxwikiPresence;
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::util::{invalid, parse, schema};
use crate::commands::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::link_check::{check_links_in, LinkWorld};

/// The capability.
pub const CAPABILITY: &str = "knowledge";
/// oxwiki's built-in entry (`capabilities::BUILT_INS`), which
/// `oxplow-foundation` declares under the id `oxwiki`.
pub const BUILT_IN: &str = "oxplow:oxwiki";

pub const WRITE_PAGE: &str = "oxplow.knowledge.write_page";
pub const DELETE_PAGE: &str = "oxplow.knowledge.delete_page";
pub const LINK: &str = "oxplow.knowledge.link";
pub const RESYNC: &str = "oxplow.knowledge.resync";
pub const VERIFY_FILES: &str = "oxplow.knowledge.verify_files";

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
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

/// Staged built-in stores. Preparing these values has no effect on the live
/// stores or oxwiki's watch, so a generation can publish them with its other
/// prepared capability state.
pub struct PreparedKnowledge {
    providers: Vec<Arc<dyn KnowledgeProvider>>,
}

impl PreparedKnowledge {
    /// Publish the declared stores; oxwiki is present when it is one of
    /// them (its watch scans its files when it appears).
    pub(crate) fn publish_maps(self, stores: &KnowledgeStores, oxwiki: &OxwikiPresence) {
        oxwiki.set(self.providers.iter().any(|p| p.id() == OXWIKI));
        stores.set_declared(self.providers);
    }
}

/// Prepare oxwiki's store for its declaration — only under its own id,
/// which its pages' URIs carry. This does not change the live stores.
pub fn prepare_built_ins(
    declared: &[crate::capabilities::Implementation],
    project_dir: &Path,
) -> PreparedKnowledge {
    let providers = declared
        .iter()
        .filter(|i| i.capability == CAPABILITY && i.id == OXWIKI)
        .filter(|i| matches!(i.source, crate::capabilities::Source::BuiltIn(BUILT_IN)))
        .take(1)
        .map(|_| -> Arc<dyn KnowledgeProvider> {
            Arc::new(oxplow_oxwiki::OxwikiStore {
                project_dir: project_dir.to_path_buf(),
            })
        })
        .collect();
    PreparedKnowledge { providers }
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
                    target: oxplow_domain::uri::build::file_uri(&target),
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

/// Marks a page touched by the thread whose command wrote it
/// (`knowledge_page_touch`, `v_knowledge_touch`), from
/// `knowledge.page.written`. A sync (no thread) marks nothing.
pub struct KnowledgeAttribution;

impl KnowledgeAttribution {
    pub const NAME: &'static str = "knowledge.attribution";
}

impl crate::event_pump::EventConsumer for KnowledgeAttribution {
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
        let written: KnowledgePageWrittenV3 =
            serde_json::from_value(event.envelope.payload.clone()).map_err(|e| {
                DomainError::Invalid(format!("knowledge.page.written payload: {e}"))
            })?;
        // A page deleted since is no longer there to mark.
        if oxplow_db::knowledge_page_store::get_tx(conn, &written.page)?.is_none() {
            return Ok(());
        }
        oxplow_db::knowledge_page_store::touch_tx(conn, thread, &written.page, event.envelope.at)
    }
}

/// Log an agent's read of the page `uri` through a tool (`oxplow_read`):
/// `knowledge.page.read { via: tool }`, as the actor, anchored to its open
/// turn (so a request that read a page is one). A read command runs
/// unrecorded, so it appends this itself.
pub async fn log_read(
    svc: &crate::Services,
    actor: &oxplow_domain::Actor,
    uri: &str,
) -> Result<(), DomainError> {
    let Some((store, id)) = oxplow_domain::uri::build::knowledge_of_uri(uri) else {
        return Ok(());
    };
    let page = knowledge_uri(&store, &id);
    let (vocabulary, source, actor) = (svc.vocabulary.clone(), actor.source(), actor.clone());
    svc.db
        .transaction(move |tx| {
            let anchors = match &actor {
                oxplow_domain::Actor::Agent {
                    thread_id: Some(thread),
                    session_id,
                    ..
                } => oxplow_db::agent_stores::activity_anchors_tx(tx, *thread, *session_id)?,
                other => other.anchors(),
            };
            let vocabulary = vocabulary.current();
            let env = EventCtx {
                vocabulary: &vocabulary,
                source: source.clone(),
                cause: None,
            }
            .typed::<KnowledgePageRead>(&KnowledgePageReadV1 {
                page: page.clone(),
                via: ReadVia::Tool,
            })
            .with_anchors(anchors)
            .with_subject([page.clone()]);
            oxplow_db::event_log_store::append_tx(tx, &vocabulary, &env).map(|_| ())
        })
        .await
}

/// An agent reading a page's file with its own tool reads the page: each
/// `agent.tool.finished` of a file read whose path an enabled store says is
/// a recorded page's file (`page_at`; oxwiki's `.oxplow/oxwiki/<slug>.md`)
/// logs `knowledge.page.read { via: file }`, caused by the tool call, with
/// its anchors. A path is the worktree's (relative) or absolute; one that is
/// no page is nothing. Every read then reaches the record the same way
/// ([`KnowledgeReads`]).
pub struct KnowledgeFileReads {
    pub stores: Arc<KnowledgeStores>,
    pub project_dir: PathBuf,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

impl KnowledgeFileReads {
    pub const NAME: &'static str = "knowledge.file_reads";
}

impl crate::event_pump::EventConsumer for KnowledgeFileReads {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <AgentToolFinished as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let Ok(tool) =
            serde_json::from_value::<AgentToolFinishedV4>(event.envelope.payload.clone())
        else {
            return Ok(());
        };
        if tool.kind != oxplow_domain::agent::tool::ToolKind::Read || tool.paths.is_empty() {
            return Ok(());
        }
        // A relative path is the worktree's the agent ran in.
        let worktree: PathBuf = match event.envelope.anchors.stream_id {
            Some(stream) => conn
                .query_row(
                    "SELECT worktree_path FROM streams WHERE id = ?1",
                    [stream.value()],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(sql)?
                .map_or_else(|| self.project_dir.clone(), PathBuf::from),
            None => self.project_dir.clone(),
        };
        let stores: Vec<Arc<dyn KnowledgeProvider>> = self
            .stores
            .enabled()
            .iter()
            .filter_map(|id| self.stores.get(id))
            .collect();
        let vocabulary = self.vocabulary.current();
        let ev = EventCtx {
            vocabulary: &vocabulary,
            source: event.envelope.source.clone(),
            cause: Some(event.envelope.id.clone()),
        };
        for path in &tool.paths {
            let path = Path::new(path);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                worktree.join(path)
            };
            for store in &stores {
                let Some(id) = store.page_at(&path) else {
                    continue;
                };
                let page = knowledge_uri(store.id(), &id);
                if oxplow_db::knowledge_page_store::get_tx(conn, &page)?.is_none() {
                    continue;
                }
                let env = ev
                    .typed::<KnowledgePageRead>(&KnowledgePageReadV1 {
                        page: page.clone(),
                        via: ReadVia::File,
                    })
                    .with_anchors(event.envelope.anchors.clone())
                    .with_subject([page]);
                ev.append(conn, &env)?;
            }
        }
        Ok(())
    }
}

/// Says which recorded pages a snapshot of the primary stream drifted: one
/// `knowledge.page.drifted` per page citing a file the take changed (its
/// `file_snapshot` rows), caused by the `snapshot.taken`. Staleness is the
/// primary stream's (`v_knowledge_pin`), so another stream's take is
/// nothing; so is a take that changed no cited file. What counts stale
/// pages (bundled's `oxplow.knowledge_pages`) follows these instead of every
/// snapshot.
pub struct KnowledgeDrift {
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

impl KnowledgeDrift {
    pub const NAME: &'static str = "knowledge.drift";
}

impl crate::event_pump::EventConsumer for KnowledgeDrift {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <SnapshotTaken as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let Ok(taken) = serde_json::from_value::<SnapshotTakenV3>(event.envelope.payload.clone())
        else {
            return Ok(());
        };
        if taken.unchanged || taken.file_count == 0 {
            return Ok(());
        }
        let mut stmt = conn
            .prepare_cached(
                "SELECT pr.source_uri, fs.path
                   FROM snapshot s
                   JOIN streams st ON st.id = s.stream_id AND st.kind = 'primary'
                   JOIN file_snapshot fs ON fs.snapshot_id = s.id
                   JOIN page_ref pr ON pr.source_resource = 'oxplow/knowledge'
                                   AND pr.target_resource = 'oxplow/file'
                                   AND uri_param(pr.target_uri, 'path') = fs.path
                   JOIN knowledge_page kp ON kp.uri = pr.source_uri
                  WHERE s.id = ?1
                  ORDER BY pr.source_uri, fs.path",
            )
            .map_err(sql)?;
        let mut drifted: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for row in stmt
            .query_map([taken.snapshot], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql)?
        {
            let (page, path) = row.map_err(sql)?;
            let paths = drifted.entry(page).or_default();
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        if drifted.is_empty() {
            return Ok(());
        }
        let vocabulary = self.vocabulary.current();
        let ev = EventCtx {
            vocabulary: &vocabulary,
            source: event.envelope.source.clone(),
            cause: Some(event.envelope.id.clone()),
        };
        for (page, paths) in drifted {
            let env = ev
                .typed::<KnowledgePageDrifted>(&KnowledgePageDriftedV1 {
                    page: page.clone(),
                    snapshot: taken.snapshot,
                    paths,
                })
                .with_anchors(event.envelope.anchors.clone())
                .with_subject([page]);
            ev.append(conn, &env)?;
        }
        Ok(())
    }
}

/// Records every read of a knowledge page an agent made
/// (`knowledge_page_read`, `v_knowledge_read`) from its
/// `knowledge.page.read`, with its thread and turn, for recorded pages
/// only.
pub struct KnowledgeReads;

impl KnowledgeReads {
    pub const NAME: &'static str = "knowledge.reads";
}

impl crate::event_pump::EventConsumer for KnowledgeReads {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <KnowledgePageRead as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let read: KnowledgePageReadV1 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("knowledge.page.read payload: {e}")))?;
        let anchors = &event.envelope.anchors;
        conn.execute(
            "INSERT INTO knowledge_page_read (uri, thread_id, turn_id, via, at)
             SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM knowledge_page WHERE uri = ?1)",
            params![
                read.page,
                anchors.thread_id.map(|t| t.value()),
                anchors.turn_id,
                read.via.as_str(),
                oxplow_db::ts_to_string(event.envelope.at),
            ],
        )
        .map_err(sql)?;
        Ok(())
    }
}

/// What the commands run against: the enabled stores, and core's record.
#[derive(Clone)]
pub struct KnowledgeTarget {
    pub project_dir: PathBuf,
    pub vcs: Arc<dyn Vcs>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub stores: Arc<KnowledgeStores>,
    pub oxwiki: Arc<OxwikiPresence>,
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

/// The store a write names, refused unless oxplow may write its pages.
fn writable(
    t: &KnowledgeTarget,
    store: Option<&str>,
) -> Result<Arc<dyn KnowledgeProvider>, CommandError> {
    let provider = t.stores.resolve(store)?;
    if !provider.features().edit {
        return Err(invalid(
            "/store",
            format!(
                "the knowledge store `{}` is read-only: oxplow doesn't write its pages",
                provider.id()
            ),
        ));
    }
    Ok(provider)
}

/// Refuse the links `body` adds that don't resolve, naming each. A link the
/// page already has (`page`'s recorded body) isn't the writer's to fix: a
/// cited file or task that has since gone, or a hand edit's bad link,
/// mustn't block verifying, rewriting or linking the page.
fn links_resolve(
    target: &KnowledgeTarget,
    conn: &rusqlite::Connection,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
    page: Option<&str>,
    body: &str,
) -> Result<(), CommandError> {
    let graph = target.vcs.revision_graph(&target.project_dir);
    let existing: std::collections::HashSet<String> = match page {
        Some(page) => recorded_body_tx(conn, page)?
            .map(|current| {
                oxplow_domain::uri::classify_wikilinks(kinds, &current)
                    .into_iter()
                    .map(|l| l.raw)
                    .collect()
            })
            .unwrap_or_default(),
        None => Default::default(),
    };
    let mut warnings = check_links_in(
        &LinkWorld {
            conn,
            resources: kinds,
            project_dir: &target.project_dir,
            graph: &*graph,
            this_page: page,
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

/// The body core recorded for the page `uri`, if it records the page.
fn recorded_body_tx(
    conn: &rusqlite::Connection,
    uri: &str,
) -> Result<Option<String>, CommandError> {
    conn.query_row(
        "SELECT body FROM knowledge_page WHERE uri = ?1",
        [uri],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| CommandError::Failed {
        message: e.to_string(),
    })
}

/// Write a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WritePageInput {
    /// The store that keeps it (`oxwiki`, an extension's): needed only when
    /// several are enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// The store's page id (oxwiki: kebab-case, `vcs-capability`, the page
    /// `.oxplow/oxwiki/<id>.md`). Omit it to have the store derive one from
    /// the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Sets the body's `# ` heading (added when it has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The whole page, markdown. Every `[[link]]` must resolve.
    #[schemars(extend("contentMediaType" = "text/markdown"))]
    pub body: String,
    /// Files you re-read against this body: their pins move to now. A
    /// file under a `[[dir:…]]` the body cites counts.
    #[serde(default)]
    pub verified_files: Vec<String>,
    /// Files you took out of the body.
    #[serde(default)]
    pub removed_files: Vec<String>,
}

/// Re-pin a page's refs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyFilesInput {
    /// The page's store: needed only when several are enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// The store's page id.
    pub id: String,
    /// The files you re-read against the page as it stands: their pins move
    /// to now. Omit for every file the page cites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
}

/// A page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    /// The page's store: needed only when several are enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// The store's page id.
    pub id: String,
}

/// An oxwiki page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResyncInput {
    /// The page's slug (`.oxplow/oxwiki/<id>.md`).
    pub id: String,
}

/// Add a link to a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkInput {
    /// The page's store: needed only when several are enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// The store's page id.
    pub id: String,
    /// What to link to, as a wikilink's inside: `other-page`, `vault:Notes/a.md`,
    /// `src/lib.rs`, `dir:src/vcs`, `tsk42`, `git:<sha>`.
    pub target: String,
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

/// Record what `store` kept of a write: its row, links, pins and
/// `knowledge.page.written`, returned for the bus to log after the run.
async fn record(
    target: &KnowledgeTarget,
    actor: &oxplow_domain::Actor,
    store: &str,
    kept: Kept,
    verified: Vec<String>,
    removed: Vec<String>,
) -> Result<HandlerOutput, CommandError> {
    let (store, vocabulary) = (store.to_string(), target.vocabulary.clone());
    let (source, anchors) = (actor.source(), actor.anchors());
    let Kept {
        id,
        body,
        title,
        kind,
        url,
        author,
        refs,
        mut events,
    } = kept;
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
                &PageWrite {
                    title: title.as_deref(),
                    kind: kind.as_deref(),
                    url: url.as_deref(),
                    author: author.as_deref(),
                    verified: &verified,
                    removed: &removed,
                    stated: refs.as_deref(),
                    anchors: anchors.clone(),
                    ..PageWrite::new(&store, &id, &body)
                },
            )
        })
        .await
        .map_err(domain)?;
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

fn call(invocation: &Invocation) -> PageCall<'_> {
    PageCall {
        run: &invocation.run,
        idempotency_key: invocation.idempotency_key.clone(),
    }
}

/// Write a page through its store, checked against core's record and
/// recorded as the store kept it.
async fn write_page(
    t: &KnowledgeTarget,
    invocation: Invocation,
    input: WritePageInput,
) -> Result<HandlerOutput, CommandError> {
    let store = writable(t, input.store.as_deref())?;
    let mut body = strip_body_version_literals(&input.body);
    if let Some(title) = &input.title {
        body = with_title(&body, title);
    }
    {
        let page = input.id.as_deref().map(|id| knowledge_uri(store.id(), id));
        let (t2, body) = (t.clone(), body.clone());
        let (verified, removed) = (input.verified_files.clone(), input.removed_files.clone());
        check(t, move |conn, vocabulary| {
            links_resolve(&t2, conn, &vocabulary.resources, page.as_deref(), &body)?;
            CitedRefs::of_body(&vocabulary.resources, &body)
                .bear_out(&verified, &removed)
                .map_err(|(field, message)| invalid(field, message))
        })
        .await?;
    }
    let draft = PageDraft {
        id: input.id.clone(),
        title: input.title.clone(),
        body,
        verified_files: input.verified_files.clone(),
        removed_files: input.removed_files.clone(),
    };
    let kept = store.write_page(&call(&invocation), &draft).await?;
    record(
        t,
        invocation.actor(),
        store.id(),
        kept,
        input.verified_files,
        input.removed_files,
    )
    .await
}

/// A recorded page's body, or refused naming it.
async fn recorded_body(t: &KnowledgeTarget, page: &str) -> Result<String, CommandError> {
    let uri = page.to_string();
    t.db.read(move |conn| Ok(recorded_body_tx(conn, &uri)))
        .await
        .map_err(CommandError::from)??
        .ok_or_else(|| {
            let (store, id) = oxplow_domain::uri::build::knowledge_of_uri(page).unwrap_or_default();
            invalid("/id", format!("no page `{id}` in `{store}`"))
        })
}

/// The page commands: each is checked against core's record (the id, every
/// link it adds, the files it verifies or removes), then dispatched to the
/// page's store, and what the store kept is recorded the same way for every
/// store. `oxplow.knowledge.resync` is oxwiki's: it restates a page from its
/// file.
pub fn ops(target: KnowledgeTarget) -> Vec<Op> {
    let t = target.clone();
    let write = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: WritePageInput = parse(input)?;
            write_page(&t, invocation, input).await
        }) as crate::commands::ExternalFuture
    }));

    let t = target.clone();
    let verify_files = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: VerifyFilesInput = parse(input)?;
        // Any enabled store's page: only its pins move, so a read-only
        // store's pages are verified too.
        let store = t.stores.resolve(input.store.as_deref())?;
        let page = knowledge_uri(store.id(), &input.id);
        let (written, env) = oxplow_db::knowledge_record::verify_tx(
            ctx.conn,
            &ctx.events,
            &page,
            input.files.as_deref(),
            ctx.actor().anchors(),
        )
        .map_err(|e| match e {
            DomainError::Invalid(message) if message.starts_with("no page") => {
                invalid("/id", format!("no page `{}` in `{}`", input.id, store.id()))
            }
            DomainError::Invalid(message) => invalid("/files", message),
            other => CommandError::from(other),
        })?;
        Ok(output(
            serde_json::to_value(&written).expect("Written serializes"),
            vec![env],
        ))
    }));

    let t = target.clone();
    let delete = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: PageInput = parse(input)?;
            let store = writable(&t, input.store.as_deref())?;
            let page = knowledge_uri(store.id(), &input.id);
            recorded_body(&t, &page).await?;
            let mut events = store.delete_page(&call(&invocation), &input.id).await?;
            let (vocabulary, source, anchors) = (
                t.vocabulary.clone(),
                invocation.actor().source(),
                invocation.actor().anchors(),
            );
            let deleted = {
                let page = page.clone();
                t.db.transaction(move |tx| {
                    let vocabulary = vocabulary.current();
                    let ev = EventCtx {
                        vocabulary: &vocabulary,
                        source: source.clone(),
                        cause: None,
                    };
                    delete_page_tx(tx, &ev, anchors.clone(), &page)
                })
                .await
                .map_err(domain)?
            };
            events.extend(deleted);
            Ok(output(json!({ "page": page }), events))
        }) as crate::commands::ExternalFuture
    }));

    let t = target.clone();
    let link = Handler::External(Arc::new(move |invocation: Invocation, input| {
        let t = t.clone();
        Box::pin(async move {
            let input: LinkInput = parse(input)?;
            let store = writable(&t, input.store.as_deref())?;
            let page = knowledge_uri(store.id(), &input.id);
            let current = recorded_body(&t, &page).await?;
            let line = format!("- [[{}]]", input.target);
            if current.lines().any(|l| l.trim() == line) {
                return Err(invalid(
                    "/target",
                    format!("`{}` already links to it", input.id),
                ));
            }
            {
                let (t2, page, line) = (t.clone(), page.clone(), line.clone());
                check(&t, move |conn, vocabulary| {
                    links_resolve(&t2, conn, &vocabulary.resources, Some(&page), &line)
                })
                .await?;
            }
            write_page(
                &t,
                invocation,
                WritePageInput {
                    store: Some(store.id().to_string()),
                    id: Some(input.id),
                    title: None,
                    body: with_related(&current, &line),
                    verified_files: Vec::new(),
                    removed_files: Vec::new(),
                },
            )
            .await
        }) as crate::commands::ExternalFuture
    }));

    let t = target;
    let resync = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: ResyncInput = parse(input)?;
        if !oxplow_oxwiki::valid_slug(&input.id) {
            return Err(invalid(
                "/id",
                format!("`{}` isn't an oxwiki page id (kebab-case)", input.id),
            ));
        }
        if !t.oxwiki.present() {
            return Err(invalid(
                "/id",
                "oxwiki is turned off: there is no file to restate from",
            ));
        }
        let page = knowledge_uri(OXWIKI, &input.id);
        let path = oxplow_oxwiki::page_path(&t.project_dir, &input.id);
        let (result, events) = match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let env = delete_page_tx(ctx.conn, &ctx.events, ctx.actor().anchors(), &page)
                    .map_err(domain)?;
                (
                    json!({ "page": page, "deleted": true }),
                    env.into_iter().collect(),
                )
            }
            Err(e) => {
                return Err(CommandError::Failed {
                    message: format!("read the page's file: {e}"),
                });
            }
            Ok(raw) => {
                let body = strip_body_version_literals(&raw);
                let (written, env) = write_page_tx(
                    ctx.conn,
                    &ctx.events,
                    &PageWrite {
                        updated_at: Timestamp::now(),
                        anchors: ctx.actor().anchors(),
                        ..PageWrite::new(OXWIKI, &input.id, &body)
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
            "verify_files",
            schema::<VerifyFilesInput>(),
            false,
            verify_files,
        ),
        Op::new(
            "knowledge.write",
            "delete_page",
            schema::<PageInput>(),
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
            schema::<ResyncInput>(),
            false,
            resync,
        ),
    ]
}

#[cfg(test)]
mod tests;
