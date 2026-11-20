//! Pure projections from per-source data into [`PageRefEdge`]s.
//!
//! Each writer that owns a source calls one of these helpers to compute
//! the edges its row contributes, then hands the result to
//! [`SqlitePageRefStore::replace_source`]. Keeping the transformation
//! pure (no DB, no IO) makes it trivially unit-testable and lets the
//! boot-time backfill replay the exact same mapping from existing rows.
//!
//! Every end is a URI (`.context/resources.md`), built here through
//! `uri::build`, or by the registry for a shared model's record (a task, a
//! page): a file's, a directory's, a finding's, a commit's, a thread note's.

use crate::page_ref_store::PageRefEdge;
use oxplow_domain::uri::build;
use oxplow_domain::uri::resources::{self as r, ResourceRegistry};
use oxplow_domain::uri::{extract, RefVersion};

/// A text body (a record's) → a file or directory it names.
pub const RT_BODY_FILE: &str = "file_mention";
pub const RT_BODY_DIR: &str = "dir_mention";
/// A text body → a record of a shared model it names (a task, a page).
pub const RT_BODY_RECORD: &str = "record_mention";
pub const RT_BODY_FINDING: &str = "finding_mention";
pub const RT_BODY_COMMIT: &str = "commit_mention";
/// A pinned markdown column's file and directory refs (`.context/models.md`
/// "Markdown bodies"): pinned to a snapshot, read by `v_ref_pin`.
pub const RT_FILE_PIN: &str = "file_pin";
pub const RT_DIR_PIN: &str = "dir_pin";
pub const RT_TOUCHED_FILE: &str = "touched_file";
pub const RT_FINDING_PATH: &str = "finding_path";
/// An agent turn → what its prompt pointed at (`@path`, `[oxplow uri …]`,
/// `[oxplow page …]`): written by the hook ingest as the prompt is
/// delivered, a reprompt adding to its turn's.
pub const RT_PROMPT_MENTION: &str = "prompt_mention";

/// Snapshot-pinned version data for a file reference. The
/// store/service layer computes this from a snapshot id at capture
/// time and stamps it onto every per-file ref row.
#[derive(Debug, Clone, Copy)]
pub struct FileRefVersion<'a> {
    pub local_snapshot_id: i64,
    pub closest_vcs_rev: Option<&'a str>,
    pub vcs_rev_exact: bool,
}

/// Owned variant of [`FileRefVersion`] for callers that need to move
/// the triple into a `'static` transaction closure.
#[derive(Debug, Clone)]
pub struct OwnedFileRefVersion {
    pub local_snapshot_id: i64,
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
}

impl OwnedFileRefVersion {
    pub fn as_ref(&self) -> FileRefVersion<'_> {
        FileRefVersion {
            local_snapshot_id: self.local_snapshot_id,
            closest_vcs_rev: self.closest_vcs_rev.as_deref(),
            vcs_rev_exact: self.vcs_rev_exact,
        }
    }
}

/// Stamp the supplied file-version triple onto every edge in
/// `edges` whose target is a file or directory. Mutates in place
/// because `with_version` consumes `self`. No-op for non-file
/// targets — page↔task, page↔page, etc. don't carry a content
/// version.
pub fn stamp_file_versions(edges: &mut [PageRefEdge], version: FileRefVersion<'_>) {
    for edge in edges.iter_mut() {
        let target = build::resource_of(&edge.target);
        if !matches!(target.as_deref(), Some(r::FILE) | Some(r::DIR)) {
            continue;
        }
        edge.local_snapshot_id = Some(version.local_snapshot_id);
        edge.closest_vcs_rev = version.closest_vcs_rev.map(|s| s.to_string());
        edge.vcs_rev_exact = version.vcs_rev_exact;
    }
}

/// A row's comment slice: its comments' mentions, recorded as the row's
/// own edges (a comment lives on its row's page).
pub fn comment_ref_types() -> Vec<String> {
    COMMENT_MENTIONS.all()
}

pub const RT_COMMENT_FILE: &str = "comment_file_ref";
pub const RT_COMMENT_DIR: &str = "comment_dir_ref";
pub const RT_COMMENT_RECORD: &str = "comment_record_mention";
pub const RT_COMMENT_FINDING: &str = "comment_finding_mention";
pub const RT_COMMENT_COMMIT: &str = "comment_commit_mention";

/// The ref types a text's mentions are recorded under, by what they name.
struct MentionTypes {
    file: &'static str,
    dir: &'static str,
    record: &'static str,
    finding: &'static str,
    commit: &'static str,
}

impl MentionTypes {
    fn all(&self) -> Vec<String> {
        [self.file, self.dir, self.record, self.finding, self.commit]
            .map(str::to_string)
            .to_vec()
    }
}

const BODY_MENTIONS: MentionTypes = MentionTypes {
    file: RT_BODY_FILE,
    dir: RT_BODY_DIR,
    record: RT_BODY_RECORD,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

/// A shared model's pinned markdown column.
const PINNED_MENTIONS: MentionTypes = MentionTypes {
    file: RT_FILE_PIN,
    dir: RT_DIR_PIN,
    record: RT_BODY_RECORD,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

/// The ref types a shared model's markdown columns own on their row's
/// source: what restating its body replaces.
pub fn model_body_ref_types(pins: bool) -> Vec<String> {
    if pins { PINNED_MENTIONS } else { BODY_MENTIONS }.all()
}

/// A shared model's markdown body edges (`source` its row's URI): the
/// body mentions, with file and dir refs as pinned refs when `pins`. A
/// file edge carries the line and version the link named.
pub fn model_body_edges(
    kinds: &ResourceRegistry,
    source: &str,
    body: &str,
    pins: bool,
) -> Vec<PageRefEdge> {
    let types = if pins {
        &PINNED_MENTIONS
    } else {
        &BODY_MENTIONS
    };
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in &refs.files_detail {
        let mut edge = PageRefEdge::new(source, build::file_uri(&fd.path), types.file);
        if !(fd.version == RefVersion::Disk && fd.line.is_none()) {
            edge = edge.with_extra(
                serde_json::json!({
                    "line": fd.line,
                    "version": match &fd.version {
                        RefVersion::Disk => "disk".to_string(),
                        RefVersion::Ref(r) => format!("ref:{r}"),
                    }
                })
                .to_string(),
            );
        }
        out.push(edge);
    }
    out.extend(
        mention_edges(kinds, source, body, types)
            .into_iter()
            .filter(|e| e.ref_type != types.file),
    );
    out
}

/// A shared model's stated refs (a provider's), as the edges its markdown
/// would have given: each URI typed by what it names (a record `kinds`
/// recognizes, a file, a directory, a finding, a commit); any other is no
/// edge a row has.
pub fn model_stated_edges(
    kinds: &ResourceRegistry,
    source: &str,
    uris: &[String],
    pins: bool,
) -> Vec<PageRefEdge> {
    let types = if pins {
        &PINNED_MENTIONS
    } else {
        &BODY_MENTIONS
    };
    uris.iter()
        .filter_map(|u| {
            let parsed = oxplow_domain::uri::OxplowUri::parse(u).ok()?;
            let t = if kinds.record_of(&parsed).is_some() {
                types.record
            } else {
                match parsed.resource().to_string().as_str() {
                    r::FILE => types.file,
                    r::DIR => types.dir,
                    r::FINDING => types.finding,
                    r::COMMIT => types.commit,
                    _ => return None,
                }
            };
            Some(PageRefEdge::new(source, u, t))
        })
        .collect()
}

const COMMENT_MENTIONS: MentionTypes = MentionTypes {
    file: RT_COMMENT_FILE,
    dir: RT_COMMENT_DIR,
    record: RT_COMMENT_RECORD,
    finding: RT_COMMENT_FINDING,
    commit: RT_COMMENT_COMMIT,
};

/// Edges contributed by a text body (`source` its URI: a record's, …).
/// Single-owner source.
pub fn body_edges(kinds: &ResourceRegistry, source: &str, body: &str) -> Vec<PageRefEdge> {
    mention_edges(kinds, source, body, &BODY_MENTIONS)
}

/// A row's comment-mention edges: what its comments' bodies name.
pub fn comment_edges<'a>(
    kinds: &ResourceRegistry,
    item: &str,
    bodies: impl IntoIterator<Item = &'a str>,
) -> Vec<PageRefEdge> {
    let mut out: Vec<PageRefEdge> = Vec::new();
    for body in bodies {
        for edge in mention_edges(kinds, item, body, &COMMENT_MENTIONS) {
            if !out
                .iter()
                .any(|e| e.target == edge.target && e.ref_type == edge.ref_type)
            {
                out.push(edge);
            }
        }
    }
    out
}

/// What `text` mentions, as edges from `source` under `types`; the source
/// naming itself isn't one.
fn mention_edges(
    kinds: &ResourceRegistry,
    source: &str,
    text: &str,
    types: &MentionTypes,
) -> Vec<PageRefEdge> {
    let refs = extract(kinds, text);
    let edge = |target: String, ref_type: &str| PageRefEdge::new(source, target, ref_type);
    let mut out = Vec::new();
    out.extend(
        refs.files_detail
            .into_iter()
            .map(|fd| edge(build::file_uri(&fd.path), types.file)),
    );
    out.extend(refs.dirs.iter().map(|d| edge(build::dir_uri(d), types.dir)));
    out.extend(
        refs.records
            .into_iter()
            .filter(|t| t != source)
            .map(|t| edge(t, types.record)),
    );
    out.extend(
        refs.findings
            .iter()
            .map(|f| edge(build::finding_uri(f), types.finding)),
    );
    out.extend(
        refs.commits
            .iter()
            .map(|c| edge(build::commit_uri(c), types.commit)),
    );
    out.retain(|e| e.target != source);
    out
}

/// One edge per finding -> file. Owned by the findings writer.
pub fn finding_edges(finding_id: &str, path: &str) -> Vec<PageRefEdge> {
    vec![PageRefEdge::new(
        build::finding_uri(finding_id),
        build::file_uri(path),
        RT_FINDING_PATH,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::uri::resources::RecordIds;

    fn ids(resource: &str, store: &str, param: &str, pattern: &str, loose: bool) -> RecordIds {
        RecordIds {
            resource: resource.into(),
            store: store.into(),
            store_param: param.into(),
            key: "id".into(),
            pattern: Some(pattern.into()),
            loose,
        }
    }

    /// Foundation's resources with one loose model (`tsk<n>`) and one
    /// bracket-only (kebab-case slugs), both made up: core names neither.
    fn kinds() -> ResourceRegistry {
        oxplow_domain::uri::resources::foundation()
            .clone()
            .with_record_ids(ids("acme/job", "jobs", "queue", r"tsk\d+", true))
            .unwrap()
            .with_record_ids(ids(
                "acme/page",
                "wiki",
                "store",
                "[a-z0-9]+(-[a-z0-9]+)*",
                false,
            ))
            .unwrap()
    }

    fn job(id: &str) -> String {
        build::of("acme/job", &[("id", id), ("queue", "jobs")])
    }

    fn page(id: &str) -> String {
        build::of("acme/page", &[("id", id), ("store", "wiki")])
    }

    #[test]
    fn a_body_mentions_each_kind_and_records_are_one_kind() {
        let edges = model_body_edges(
            &kinds(),
            &job("tsk1"),
            "see [[src/app.rs]] [[dir:src]] [[architecture]] tsk2 [[finding:fnd-1]] [[git:abcdef0]] tsk1",
            false,
        );
        let got: std::collections::BTreeSet<(String, String)> = edges
            .iter()
            .map(|e| (e.target.clone(), e.ref_type.clone()))
            .collect();
        for (target, ty) in [
            (build::file_uri("src/app.rs"), RT_BODY_FILE),
            (build::dir_uri("src"), RT_BODY_DIR),
            (page("architecture"), RT_BODY_RECORD),
            (job("tsk2"), RT_BODY_RECORD),
            (build::finding_uri("fnd-1"), RT_BODY_FINDING),
            (build::commit_uri("abcdef0"), RT_BODY_COMMIT),
        ] {
            assert!(
                got.contains(&(target.clone(), ty.to_string())),
                "{target} {ty}: {got:?}"
            );
        }
        // A row naming itself isn't an edge.
        assert!(edges.iter().all(|e| e.target != job("tsk1")));
        assert!(edges.iter().all(|e| e.source == job("tsk1")));
    }

    /// Comments' mentions are the row's own edges, under the comment ref
    /// types, each target once.
    #[test]
    fn comment_edges_are_the_rows_under_comment_types() {
        let edges = comment_edges(
            &kinds(),
            &job("tsk1"),
            ["see [[src/app.rs]] and tsk2", "again [[src/app.rs]]"],
        );
        let got: Vec<(String, String, &str)> = edges
            .iter()
            .map(|e| (e.source.clone(), e.target.clone(), e.ref_type.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (job("tsk1"), build::file_uri("src/app.rs"), RT_COMMENT_FILE),
                (job("tsk1"), job("tsk2"), RT_COMMENT_RECORD),
            ]
        );
    }

    /// A provider's stated refs are typed by what they name: a record its
    /// models hold, a file; a resource that isn't a row's link is no edge.
    #[test]
    fn stated_refs_are_typed_by_what_they_name() {
        let edges = model_stated_edges(
            &kinds(),
            &job("tsk1"),
            &[
                page("intro"),
                build::file_uri("a.rs"),
                build::thread_uri(oxplow_domain::ThreadId::new(1)),
            ],
            false,
        );
        let got: Vec<(&str, &str)> = edges
            .iter()
            .map(|e| (e.target.as_str(), e.ref_type.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (page("intro").as_str(), RT_BODY_RECORD),
                (build::file_uri("a.rs").as_str(), RT_BODY_FILE),
            ]
        );
    }

    #[test]
    fn finding_edges_point_at_file() {
        let edges = finding_edges("fnd-7", "src/app.rs");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source, build::finding_uri("fnd-7"));
        assert_eq!(edges[0].target, build::file_uri("src/app.rs"));
    }
}
