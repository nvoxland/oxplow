//! Core's record of knowledge pages, the same for every store
//! (`.context/knowledge.md` "The record"): a page's row, its `page_ref`
//! edges (new file refs pinned to the primary stream's latest snapshot) and
//! its `knowledge.page.written` / `deleted`, restated from what a store kept.
//! The commands, oxwiki's file sync and a provider's streamed records all
//! write through [`write_page_tx`] and [`delete_page_tx`].

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use oxplow_domain::events::schema::{
    KnowledgePageDeleted, KnowledgePageDeletedV3, KnowledgePageWritten, KnowledgePageWrittenV3,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::uri::build::{file_of_uri, file_uri, parse_resource};
use oxplow_domain::uri::resources::{self as r, ResourceRegistry};
use oxplow_domain::vcs::Revision;
use oxplow_domain::{Anchors, DomainError, Timestamp};

use crate::knowledge_page_store::{self, KnowledgePage};
use crate::page_ref_projections::{
    knowledge_edges, stamp_file_versions, stated_edges, FileRefVersion, RT_KNOWLEDGE_FILE,
};
use crate::page_ref_store::{merge_source_tx, replace_source_tx, upsert_edge_tx, PageRefEdge};
use crate::EventCtx;

fn sql(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}

/// What new file refs are pinned to: the primary stream's latest snapshot,
/// and the revision that snapshot is exactly, when it is one. Freshness
/// compares the pinned snapshot to a file's latest (`oxplow_knowledge_drift`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pin {
    pub snapshot_id: Option<i64>,
    pub revision: Option<String>,
}

impl Pin {
    fn stamp(&self, edge: PageRefEdge) -> PageRefEdge {
        match self.snapshot_id {
            Some(id) => edge.with_version(id, self.revision.clone(), self.revision.is_some()),
            None => edge,
        }
    }
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

/// The files and directories a page cites.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CitedRefs {
    /// Workspace-relative file paths (`src/foo.rs`).
    pub files: Vec<String>,
    /// Workspace-relative directory paths (`src/components`).
    pub dirs: Vec<String>,
}

impl CitedRefs {
    /// What a body in oxplow's markdown cites (`[[…]]` links and inline
    /// paths, outside code).
    pub fn of_body(resources: &ResourceRegistry, body: &str) -> Self {
        let refs = oxplow_domain::uri::extract(resources, body);
        Self {
            files: refs.files,
            dirs: refs.dirs,
        }
    }

    /// What a store states its page links (a provider's record), as URIs.
    pub fn stated(uris: &[String]) -> Self {
        let of = |resource: &str| -> Vec<String> {
            let set: std::collections::BTreeSet<String> = uris
                .iter()
                .filter_map(|u| {
                    parse_resource(u, resource)
                        .ok()?
                        .param("path")
                        .filter(|p| !p.is_empty())
                        .map(str::to_string)
                })
                .collect();
            set.into_iter().collect()
        };
        Self {
            files: of(r::FILE),
            dirs: of(r::DIR),
        }
    }

    /// Refuse `verified` / `removed` files the page doesn't bear out: a
    /// removed file still cited, a verified one neither cited nor under a
    /// cited directory. The error names the field and the files.
    pub fn bear_out(
        &self,
        verified: &[String],
        removed: &[String],
    ) -> Result<(), (&'static str, String)> {
        let still_cited: Vec<&str> = removed
            .iter()
            .filter(|p| self.files.contains(p))
            .map(String::as_str)
            .collect();
        if !still_cited.is_empty() {
            return Err((
                "/removed_files",
                format!(
                    "removed_files still cited by the body: {}",
                    still_cited.join(", ")
                ),
            ));
        }
        let unreferenced: Vec<&str> = verified
            .iter()
            .filter(|p| !self.files.contains(p) && !path_under_any_dir(p, &self.dirs))
            .map(String::as_str)
            .collect();
        if !unreferenced.is_empty() {
            return Err((
                "/verified_files",
                format!(
                    "verified_files neither cited by the body nor under a cited directory: {}",
                    unreferenced.join(", ")
                ),
            ));
        }
        Ok(())
    }
}

/// True if `path` is a file located under any of `dirs`. Workspace-relative,
/// `/`-separated; an exact equality is not a match (a file path can't equal
/// a directory).
pub fn path_under_any_dir(path: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| {
        let prefix = format!("{}/", d.trim_end_matches('/'));
        path.starts_with(&prefix)
    })
}

/// A body's first `# ` heading, else `fallback`.
pub fn extract_title(body: &str, fallback: &str) -> String {
    body.lines()
        .filter_map(|l| l.trim_start().strip_prefix("# "))
        .map(str::trim)
        .find(|t| !t.is_empty())
        .map_or_else(|| fallback.to_string(), str::to_string)
}

/// The hash a page row keeps of the body it was written from.
pub fn body_hash(body: &str) -> String {
    use xxhash_rust::xxh3::Xxh3;
    let mut h = Xxh3::new();
    h.update(body.as_bytes());
    format!("{:032x}", h.digest128())
}

/// Whether `target` (a wikilink's, before an `@`) is a file or directory
/// link, whose `@…` is a version: no `<prefix>:`, or `file:` / `dir:`.
fn versioned(target: &str) -> bool {
    match target.split_once(':') {
        None => true,
        Some((prefix, _)) => matches!(prefix.trim(), "file" | "dir"),
    }
}

/// Strip `@<version>` suffixes from `[[…]]` wikilinks (keeping a `|label`)
/// and from `(file:…)` / `(dir:…)` markdown link targets. A page's prose
/// names what it relies on; how current that is, is the pin on its edge.
/// Other schemes (`http(s)`, `mailto:`) are left alone.
pub fn strip_body_version_literals(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        if let Some(after_open) = rest.strip_prefix("[[") {
            if let Some(close) = after_open.find("]]") {
                let inner = &after_open[..close];
                let (path_part, label_part) = match inner.find('|') {
                    Some(p) => (&inner[..p], Some(&inner[p..])),
                    None => (inner, None),
                };
                let stripped = match path_part.find('@') {
                    // A file's or a directory's version: `[[src/a.rs@HEAD]]`,
                    // `[[file:a.rs@x]]`. An `@` inside another link's target
                    // (`[[vault:people/me@home]]`) is part of its id.
                    Some(at) if versioned(&path_part[..at]) => &path_part[..at],
                    _ => path_part,
                };
                out.push_str("[[");
                out.push_str(stripped);
                if let Some(label) = label_part {
                    out.push_str(label);
                }
                out.push_str("]]");
                i += 2 + close + 2;
                continue;
            }
        }
        if let Some(after_paren) = rest.strip_prefix('(') {
            let scheme_len = match after_paren.split_once(':').map(|(scheme, _)| scheme) {
                Some(scheme @ ("file" | "dir")) => Some(scheme.len() + 1),
                _ => None,
            };
            if let (Some(scheme_len), Some(close)) = (scheme_len, after_paren.find(')')) {
                let url = &after_paren[..close];
                if scheme_len <= url.len() {
                    let path = &url[scheme_len..];
                    out.push('(');
                    out.push_str(&url[..scheme_len]);
                    out.push_str(path.find('@').map_or(path, |at| &path[..at]));
                    out.push(')');
                    i += 1 + close + 1;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().expect("non-empty");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// A page to restate, as its store kept it, with the files its author
/// re-checked (`verified`: pinned to now) and dropped (`removed`: must be
/// gone from the body).
pub struct PageWrite<'a> {
    pub store: &'a str,
    /// The store's page id.
    pub id: &'a str,
    pub body: &'a str,
    /// Its title; `None`: the body's first heading, else the id.
    pub title: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub url: Option<&'a str>,
    pub author: Option<&'a str>,
    pub verified: &'a [String],
    pub removed: &'a [String],
    pub updated_at: Timestamp,
    /// The refs its store states (a provider's record); `None`: parsed
    /// from the body (oxplow's markdown).
    pub stated: Option<&'a [String]>,
    /// Who wrote it, on the event (a command's actor's thread and
    /// stream; none for a sync).
    pub anchors: Anchors,
}

impl<'a> PageWrite<'a> {
    /// A write of `body` to `store`'s page `id`, now, from a body in
    /// oxplow's markdown, by nobody in particular.
    pub fn new(store: &'a str, id: &'a str, body: &'a str) -> Self {
        Self {
            store,
            id,
            body,
            title: None,
            kind: None,
            url: None,
            author: None,
            verified: &[],
            removed: &[],
            updated_at: Timestamp::now(),
            stated: None,
            anchors: Anchors::default(),
        }
    }
}

/// What a write left: the page, its outbound refs and its pin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Written {
    /// The page's URI.
    pub page: String,
    pub store: String,
    /// The store's page id.
    pub id: String,
    pub outbound: Vec<String>,
    /// The snapshot its new refs are pinned to.
    pub snapshot: Option<i64>,
}

/// Restate a page's row and edges from its body, answering what it left
/// and its `knowledge.page.written` for the caller to log. Edges already
/// stored keep their pins (so unrelated prose edits don't re-stamp them);
/// new file refs and verified ones take the current [`Pin`]. A file
/// verified under a directory the body cites (`[[dir:…]]`) becomes an edge
/// of its own, kept while that directory stays cited.
pub fn write_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    write: &PageWrite<'_>,
) -> Result<(Written, Envelope), DomainError> {
    let uri = oxplow_domain::uri::build::knowledge_uri(write.store, write.id);
    let resources = &ev.vocabulary.resources;
    let cited = match write.stated {
        Some(stated) => CitedRefs::stated(stated),
        None => CitedRefs::of_body(resources, write.body),
    };
    cited
        .bear_out(write.verified, write.removed)
        .map_err(|(_, message)| DomainError::Invalid(message))?;
    let created_at = knowledge_page_store::get_tx(conn, &uri)?
        .map_or(write.updated_at, |(page, _)| page.created_at);
    let page = KnowledgePage {
        uri: uri.clone(),
        store: write.store.to_string(),
        id: write.id.to_string(),
        kind: write.kind.map(str::to_string),
        title: write
            .title
            .map_or_else(|| extract_title(write.body, write.id), str::to_string),
        url: write.url.map(str::to_string),
        author: write.author.map(str::to_string),
        excerpt: write.body.chars().take(280).collect(),
        body_size_bytes: write.body.len() as i64,
        file_refs: cited.files.clone(),
        dir_refs: cited.dirs.clone(),
        created_at,
        updated_at: write.updated_at,
    };
    knowledge_page_store::upsert_tx(conn, &page, write.body, &body_hash(write.body))?;

    let pin = pin_tx(conn)?;
    let mut edges = match write.stated {
        Some(stated) => stated_edges(&uri, stated),
        None => knowledge_edges(resources, &uri, write.body),
    };
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
    for kept in file_edges_tx(conn, &uri)? {
        let cited_now = edges.iter().any(|e| e.target == kept.target);
        let under = file_of_uri(&kept.target)
            .is_ok_and(|file| path_under_any_dir(&file.path, &page.dir_refs));
        if !cited_now && under {
            edges.push(kept);
        }
    }
    merge_source_tx(conn, &uri, edges.clone())?;
    for path in write.verified {
        let target = file_uri(path);
        let edge = edges
            .iter()
            .find(|e| e.target == target && e.ref_type == RT_KNOWLEDGE_FILE)
            .cloned()
            .unwrap_or_else(|| PageRefEdge::new(&uri, &target, RT_KNOWLEDGE_FILE));
        upsert_edge_tx(conn, &pin.stamp(edge))?;
    }

    let written = Written {
        page: uri.clone(),
        store: write.store.to_string(),
        id: write.id.to_string(),
        outbound: outbound_tx(conn, &uri)?,
        snapshot: pin.snapshot_id,
    };
    let env = ev
        .typed::<KnowledgePageWritten>(&KnowledgePageWrittenV3 {
            page: uri.clone(),
            store: written.store.clone(),
            outbound: written.outbound.clone(),
            snapshot: written.snapshot,
        })
        .with_anchors(Anchors {
            snapshot_id: pin.snapshot_id,
            ..write.anchors.clone()
        })
        .with_subject([uri]);
    Ok((written, env))
}

/// Re-pin files of the recorded page `uri` to now, without rewriting it:
/// `files`, or every file it cites when `None`. Each must be cited or under
/// a cited directory. Only the pins move — nothing reaches the page's store,
/// so a read-only store's pages are verified the same way. Answers what the
/// page now links and its `knowledge.page.written` for the caller to log.
pub fn verify_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    uri: &str,
    files: Option<&[String]>,
    anchors: Anchors,
) -> Result<(Written, Envelope), DomainError> {
    let Some((page, _)) = knowledge_page_store::get_tx(conn, uri)? else {
        return Err(DomainError::Invalid(format!("no page `{uri}`")));
    };
    let pinned: Vec<String> = file_edges_tx(conn, uri)?
        .iter()
        .filter_map(|e| file_of_uri(&e.target).ok().map(|f| f.path))
        .collect();
    let files: Vec<String> = files.map_or(pinned, <[String]>::to_vec);
    CitedRefs {
        files: page.file_refs.clone(),
        dirs: page.dir_refs.clone(),
    }
    .bear_out(&files, &[])
    .map_err(|(_, message)| DomainError::Invalid(message))?;
    let pin = pin_tx(conn)?;
    let edges = file_edges_tx(conn, uri)?;
    for path in &files {
        let target = file_uri(path);
        let edge = edges
            .iter()
            .find(|e| e.target == target)
            .cloned()
            .unwrap_or_else(|| PageRefEdge::new(uri, &target, RT_KNOWLEDGE_FILE));
        upsert_edge_tx(conn, &pin.stamp(edge))?;
    }
    let written = Written {
        page: uri.to_string(),
        store: page.store.clone(),
        id: page.id.clone(),
        outbound: outbound_tx(conn, uri)?,
        snapshot: pin.snapshot_id,
    };
    let env = ev
        .typed::<KnowledgePageWritten>(&KnowledgePageWrittenV3 {
            page: uri.to_string(),
            store: written.store.clone(),
            outbound: written.outbound.clone(),
            snapshot: written.snapshot,
        })
        .with_anchors(Anchors {
            snapshot_id: pin.snapshot_id,
            ..anchors
        })
        .with_subject([uri.to_string()]);
    Ok((written, env))
}

/// Delete a page's row and edges, answering its `knowledge.page.deleted`
/// for the caller to log; `None` when there was no page.
pub fn delete_page_tx(
    conn: &rusqlite::Connection,
    ev: &EventCtx<'_>,
    anchors: Anchors,
    uri: &str,
) -> Result<Option<Envelope>, DomainError> {
    let existed = knowledge_page_store::delete_tx(conn, uri)?;
    replace_source_tx(conn, uri, Vec::new())?;
    Ok(existed.then(|| {
        ev.typed::<KnowledgePageDeleted>(&KnowledgePageDeletedV3 {
            page: uri.to_string(),
        })
        .with_anchors(anchors)
        .with_subject([uri.to_string()])
    }))
}

/// The file edges a page (`source`, its URI) holds.
fn file_edges_tx(
    conn: &rusqlite::Connection,
    source: &str,
) -> Result<Vec<PageRefEdge>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT target_uri, ref_type, source_extra, local_snapshot_id, closest_vcs_rev,
                    vcs_rev_exact
             FROM page_ref WHERE source_uri = ?1 AND target_resource = ?2",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![source, r::FILE], |r| {
            let mut edge = PageRefEdge::new(source, r.get::<_, String>(0)?, r.get::<_, String>(1)?);
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

fn outbound_tx(conn: &rusqlite::Connection, uri: &str) -> Result<Vec<String>, DomainError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT target_uri FROM page_ref WHERE source_uri = ?1 ORDER BY 1")
        .map_err(sql)?;
    let rows = stmt
        .query_map(params![uri], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(sql)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_literals_are_stripped_from_links_only() {
        assert_eq!(
            strip_body_version_literals(
                "[[src/a.rs@HEAD:4|x]] (file:src/b.rs@abc) (https://h@x) [[intro]] [[vault:me@home]]"
            ),
            "[[src/a.rs|x]] (file:src/b.rs) (https://h@x) [[intro]] [[vault:me@home]]"
        );
    }

    #[test]
    fn the_title_is_the_first_heading_else_the_fallback() {
        assert_eq!(extract_title("intro\n# Arch \n# B", "x"), "Arch");
        assert_eq!(extract_title("no heading", "x"), "x");
    }

    #[test]
    fn verified_and_removed_files_are_borne_out() {
        let cited = CitedRefs {
            files: vec!["a.rs".into()],
            dirs: vec!["crates/cp".into()],
        };
        assert!(cited
            .bear_out(&["a.rs".into(), "crates/cp/x.rs".into()], &[])
            .is_ok());
        assert_eq!(
            cited.bear_out(&[], &["a.rs".into()]).unwrap_err().0,
            "/removed_files"
        );
        assert_eq!(
            cited.bear_out(&["b.rs".into()], &[]).unwrap_err().0,
            "/verified_files"
        );
    }
}
