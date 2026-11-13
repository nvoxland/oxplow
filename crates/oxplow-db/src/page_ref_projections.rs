//! Pure projections from per-source data into [`PageRefEdge`]s.
//!
//! Each writer that owns a source calls one of these helpers to compute
//! the edges its row contributes, then hands the result to
//! [`SqlitePageRefStore::replace_source`]. Keeping the transformation
//! pure (no DB, no IO) makes it trivially unit-testable and lets the
//! boot-time backfill replay the exact same mapping from existing rows.
//!
//! Every end is a URI (`.context/resources.md`), built here through
//! `uri::build`: a knowledge page's, a task's, a file's, a directory's, a
//! finding's, a commit's, a thread note's.

use crate::page_ref_store::PageRefEdge;
use oxplow_domain::uri::build;
use oxplow_domain::uri::resources::{self as r, ResourceRegistry};
use oxplow_domain::uri::{extract, RefVersion};

pub const RT_KNOWLEDGE_FILE: &str = "knowledge_file_ref";
pub const RT_KNOWLEDGE_DIR: &str = "knowledge_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
/// A text body (a task's, a handoff's) → a file or directory it names.
pub const RT_BODY_FILE: &str = "file_mention";
pub const RT_BODY_DIR: &str = "dir_mention";
pub const RT_BODY_TASK: &str = "task_mention";
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

/// A task's body slice: its title and body's mentions.
pub fn task_body_ref_types() -> Vec<String> {
    BODY_MENTIONS.all()
}

/// A task's comment slice: its comments' mentions, recorded as the
/// item's own edges (a comment lives on its item's page).
pub fn task_comment_ref_types() -> Vec<String> {
    COMMENT_MENTIONS.all()
}

pub const RT_COMMENT_FILE: &str = "comment_file_ref";
pub const RT_COMMENT_DIR: &str = "comment_dir_ref";
pub const RT_COMMENT_WIKILINK: &str = "comment_wikilink";
pub const RT_COMMENT_TASK: &str = "comment_task_mention";
pub const RT_COMMENT_FINDING: &str = "comment_finding_mention";
pub const RT_COMMENT_COMMIT: &str = "comment_commit_mention";

/// The ref types a text's mentions are recorded under, by what they name.
struct MentionTypes {
    file: &'static str,
    dir: &'static str,
    knowledge: &'static str,
    task: &'static str,
    finding: &'static str,
    commit: &'static str,
}

impl MentionTypes {
    fn all(&self) -> Vec<String> {
        [
            self.file,
            self.dir,
            self.knowledge,
            self.task,
            self.finding,
            self.commit,
        ]
        .map(str::to_string)
        .to_vec()
    }
}

const BODY_MENTIONS: MentionTypes = MentionTypes {
    file: RT_BODY_FILE,
    dir: RT_BODY_DIR,
    knowledge: RT_WIKILINK,
    task: RT_BODY_TASK,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

/// A knowledge page's body: its files and directories are what freshness
/// pins (`v_oxplow_knowledge_ref`).
const KNOWLEDGE_MENTIONS: MentionTypes = MentionTypes {
    file: RT_KNOWLEDGE_FILE,
    dir: RT_KNOWLEDGE_DIR,
    knowledge: RT_WIKILINK,
    task: RT_BODY_TASK,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

/// A shared model's pinned markdown column.
const PINNED_MENTIONS: MentionTypes = MentionTypes {
    file: RT_FILE_PIN,
    dir: RT_DIR_PIN,
    knowledge: RT_WIKILINK,
    task: RT_BODY_TASK,
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
/// would have given: each URI typed by what it names.
pub fn model_stated_edges(source: &str, uris: &[String], pins: bool) -> Vec<PageRefEdge> {
    let types = if pins {
        &PINNED_MENTIONS
    } else {
        &BODY_MENTIONS
    };
    uris.iter()
        .filter_map(|u| {
            let t = match build::resource_of(u)?.as_str() {
                r::FILE => types.file,
                r::DIR => types.dir,
                r::KNOWLEDGE => types.knowledge,
                r::TASK => types.task,
                r::FINDING => types.finding,
                r::COMMIT => types.commit,
                _ => return None,
            };
            Some(PageRefEdge::new(source, u, t))
        })
        .collect()
}

const COMMENT_MENTIONS: MentionTypes = MentionTypes {
    file: RT_COMMENT_FILE,
    dir: RT_COMMENT_DIR,
    knowledge: RT_COMMENT_WIKILINK,
    task: RT_COMMENT_TASK,
    finding: RT_COMMENT_FINDING,
    commit: RT_COMMENT_COMMIT,
};

/// Edges contributed by a knowledge page's body (`page` its URI), parsed
/// as oxplow's markdown. A file edge carries the line and version the link
/// named.
pub fn knowledge_edges(kinds: &ResourceRegistry, page: &str, body: &str) -> Vec<PageRefEdge> {
    let source = page;
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in &refs.files_detail {
        let mut edge = PageRefEdge::new(source, build::file_uri(&fd.path), RT_KNOWLEDGE_FILE);
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
        mention_edges(kinds, source, body, &KNOWLEDGE_MENTIONS)
            .into_iter()
            .filter(|e| e.ref_type != RT_KNOWLEDGE_FILE),
    );
    out
}

/// The ref type a knowledge page's stated link to `target` is recorded under,
/// by what it names; `None` for a resource that isn't a page's link.
fn stated_type(target: &str) -> Option<&'static str> {
    Some(match build::resource_of(target)?.as_str() {
        r::FILE => RT_KNOWLEDGE_FILE,
        r::DIR => RT_KNOWLEDGE_DIR,
        r::KNOWLEDGE => RT_WIKILINK,
        r::TASK => RT_BODY_TASK,
        r::FINDING => RT_BODY_FINDING,
        r::COMMIT => RT_BODY_COMMIT,
        _ => return None,
    })
}

/// Edges of a knowledge page (`page` its URI) whose store states its links
/// (a provider's record: a file's URI, another page's, a task's, …) rather
/// than a body core parses. A URI of another resource isn't an edge a page
/// has.
pub fn stated_edges(page: &str, uris: &[String]) -> Vec<PageRefEdge> {
    uris.iter()
        .filter_map(|u| Some(PageRefEdge::new(page, u, stated_type(u)?)))
        .collect()
}

/// Edges contributed by a text body (`source` its URI: a handoff's, …).
/// Single-owner source.
pub fn body_edges(kinds: &ResourceRegistry, source: &str, body: &str) -> Vec<PageRefEdge> {
    mention_edges(kinds, source, body, &BODY_MENTIONS)
}

/// A task's body-mention edges — what its title and body name — for
/// any list's item; `item` is its URI.
pub fn task_edges(
    kinds: &ResourceRegistry,
    item: &str,
    title: &str,
    body: &str,
) -> Vec<PageRefEdge> {
    mention_edges(kinds, item, &format!("{title}\n{body}"), &BODY_MENTIONS)
}

/// A task's comment-mention edges: what its comments' bodies name.
pub fn task_comment_edges<'a>(
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
    out.extend(refs.knowledge.into_iter().map(|k| edge(k, types.knowledge)));
    out.extend(
        refs.tasks
            .into_iter()
            .filter(|t| t != source)
            .map(|t| edge(t, types.task)),
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
    use oxplow_domain::uri::build::resource_of;

    /// Foundation's resources with oxtask as the task tracker (`tsk<n>`).
    fn tasks_kinds() -> oxplow_domain::uri::resources::ResourceRegistry {
        oxplow_domain::uri::resources::with_bundled_knowledge(
            oxplow_domain::uri::resources::foundation().clone(),
        )
        .with_task_ids("oxtask", r"tsk\d+")
        .unwrap()
        .with_knowledge_store("oxwiki", Some("[a-z0-9]+(-[a-z0-9]+)*"))
        .unwrap()
    }

    fn task(id: &str) -> String {
        build::task_uri("oxtask", id)
    }

    fn resource(e: &PageRefEdge) -> String {
        resource_of(&e.target).unwrap()
    }

    #[test]
    fn knowledge_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[tsk7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let intro = build::knowledge_uri("oxwiki", "intro");
        let edges = knowledge_edges(&tasks_kinds(), &intro, body);
        let kinds: std::collections::BTreeSet<_> = edges.iter().map(resource).collect();
        for k in ["file", "dir", "knowledge", "task", "finding", "commit"] {
            assert!(kinds.contains(&format!("oxplow/{k}")), "{k}: {kinds:?}");
        }
        // Every stored target is a URI foundation declares, but a task's:
        // oxplow-bundled declares that resource.
        for e in edges.iter().filter(|e| resource(e) != "oxplow/task") {
            let r = oxplow_domain::uri::OxplowUri::parse(&e.target)
                .unwrap_or_else(|err| panic!("{}: {err}", e.target));
            tasks_kinds()
                .validate(&r)
                .unwrap_or_else(|err| panic!("{}: {err}", e.target));
            assert_eq!(e.source, intro);
        }
        assert!(edges.iter().any(|e| e.target == task("tsk7")));
    }

    #[test]
    fn task_edges_parse_the_body() {
        let edges = task_edges(
            &tasks_kinds(),
            &task("tsk1"),
            "fix something",
            "see [[src/app.rs]] for context, blocked by tsk2, touches finding:fnd-9",
        );
        let targets: Vec<_> = edges.iter().map(|e| e.target.as_str()).collect();
        assert!(targets.contains(&build::file_uri("src/app.rs").as_str()));
        assert!(targets.contains(&task("tsk2").as_str()));
        assert!(targets.contains(&build::finding_uri("fnd-9").as_str()));
        // self-mention filtered out
        assert!(!targets.contains(&task("tsk1").as_str()));
        assert!(edges.iter().all(|e| e.source == task("tsk1")));
    }

    /// Comments' mentions are the item's own edges, under the comment
    /// ref types, each target once.
    #[test]
    fn comment_edges_are_the_items_under_comment_types() {
        let edges = task_comment_edges(
            &tasks_kinds(),
            &task("tsk1"),
            ["see [[src/app.rs]] and tsk2", "again [[src/app.rs]]"],
        );
        let got: Vec<(String, String, &str)> = edges
            .iter()
            .map(|e| (e.source.clone(), e.target.clone(), e.ref_type.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (task("tsk1"), build::file_uri("src/app.rs"), RT_COMMENT_FILE),
                (task("tsk1"), task("tsk2"), RT_COMMENT_TASK),
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
