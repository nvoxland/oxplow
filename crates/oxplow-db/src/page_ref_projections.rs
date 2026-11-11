//! Pure projections from per-source data into [`PageRefEdge`]s.
//!
//! Each writer that owns a source calls one of these helpers to compute
//! the edges its row contributes, then hands the result to
//! [`SqlitePageRefStore::replace_source`]. Keeping the transformation
//! pure (no DB, no IO) makes it trivially unit-testable and lets the
//! boot-time backfill replay the exact same mapping from existing rows.
//!
//! Every end is a URI (`.context/resources.md`), built here through
//! `uri::build`: a wiki page's, a task's, a file's, a directory's, a
//! finding's, a commit's, a thread note's.

use crate::page_ref_store::PageRefEdge;
use oxplow_domain::uri::build;
use oxplow_domain::uri::resources::{self as r, ResourceRegistry};
use oxplow_domain::uri::{extract, RefVersion};

pub const RT_WIKI_FILE: &str = "wiki_file_ref";
pub const RT_WIKI_DIR: &str = "wiki_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
pub const RT_BODY_TASK: &str = "task_mention";
pub const RT_BODY_FINDING: &str = "finding_mention";
pub const RT_BODY_COMMIT: &str = "commit_mention";
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
/// targets — wiki↔task, wiki↔wiki, etc. don't carry a content
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

/// A task's link slice is every ref type with this prefix — one per
/// link type, the list's own (`task_link:blocks`).
pub const RT_LINK_PREFIX: &str = "task_link:";

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
    wiki: &'static str,
    task: &'static str,
    finding: &'static str,
    commit: &'static str,
}

impl MentionTypes {
    fn all(&self) -> Vec<String> {
        [
            self.file,
            self.dir,
            self.wiki,
            self.task,
            self.finding,
            self.commit,
        ]
        .map(str::to_string)
        .to_vec()
    }
}

const BODY_MENTIONS: MentionTypes = MentionTypes {
    file: RT_WIKI_FILE,
    dir: RT_WIKI_DIR,
    wiki: RT_WIKILINK,
    task: RT_BODY_TASK,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

const COMMENT_MENTIONS: MentionTypes = MentionTypes {
    file: RT_COMMENT_FILE,
    dir: RT_COMMENT_DIR,
    wiki: RT_COMMENT_WIKILINK,
    task: RT_COMMENT_TASK,
    finding: RT_COMMENT_FINDING,
    commit: RT_COMMENT_COMMIT,
};

/// Edges contributed by a wiki page body. Owned by `wiki_pages` sync.
/// A file edge carries the line and version the link named.
pub fn wiki_edges(kinds: &ResourceRegistry, slug: &str, body: &str) -> Vec<PageRefEdge> {
    let source = build::wiki_uri(slug);
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in &refs.files_detail {
        let mut edge = PageRefEdge::new(&source, build::file_uri(&fd.path), RT_WIKI_FILE);
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
        mention_edges(kinds, &source, body, &BODY_MENTIONS)
            .into_iter()
            .filter(|e| e.ref_type != RT_WIKI_FILE),
    );
    out
}

/// The ref type a wiki page's stated link to `target` is recorded under,
/// by what it names; `None` for a resource that isn't a page's link.
fn stated_type(target: &str) -> Option<&'static str> {
    Some(match build::resource_of(target)?.as_str() {
        r::FILE => RT_WIKI_FILE,
        r::DIR => RT_WIKI_DIR,
        r::WIKI => RT_WIKILINK,
        r::TASK => RT_BODY_TASK,
        r::FINDING => RT_BODY_FINDING,
        r::COMMIT => RT_BODY_COMMIT,
        _ => return None,
    })
}

/// Edges of a wiki page whose store states its links (a provider's
/// record: a file's URI, another page's, a task's, …) rather than a
/// body core parses. A URI of another resource isn't an edge a page has.
pub fn stated_edges(slug: &str, uris: &[String]) -> Vec<PageRefEdge> {
    let source = build::wiki_uri(slug);
    uris.iter()
        .filter_map(|u| Some(PageRefEdge::new(&source, u, stated_type(u)?)))
        .collect()
}

/// Edges contributed by a note's body (`source` its URI). Single-owner
/// source.
pub fn note_edges(kinds: &ResourceRegistry, source: &str, body: &str) -> Vec<PageRefEdge> {
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

/// A task's link edges: one per `(target URI, link type)`, its ref
/// type `task_link:<type>`. Any URI: another item, or what the item
/// is about.
pub fn task_link_edges(item: &str, links: &[(String, String)]) -> Vec<PageRefEdge> {
    links
        .iter()
        .filter(|(target, _)| oxplow_domain::uri::OxplowUri::parse(target).is_ok())
        .map(|(target, link_type)| {
            PageRefEdge::new(item, target, format!("{RT_LINK_PREFIX}{link_type}"))
        })
        .collect()
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
        refs.wikis
            .iter()
            .map(|w| edge(build::wiki_uri(w), types.wiki)),
    );
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
        oxplow_domain::uri::resources::foundation()
            .clone()
            .with_task_ids("oxtask", r"tsk\d+")
            .unwrap()
    }

    fn task(id: &str) -> String {
        build::task_uri("oxtask", id)
    }

    fn resource(e: &PageRefEdge) -> String {
        resource_of(&e.target).unwrap()
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[tsk7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges(&tasks_kinds(), "intro", body);
        let kinds: std::collections::BTreeSet<_> = edges.iter().map(resource).collect();
        for k in ["file", "dir", "wiki", "task", "finding", "commit"] {
            assert!(kinds.contains(&format!("oxplow/{k}")), "{k}: {kinds:?}");
        }
        // Every stored target is a URI foundation declares.
        for e in &edges {
            let r = oxplow_domain::uri::OxplowUri::parse(&e.target)
                .unwrap_or_else(|err| panic!("{}: {err}", e.target));
            tasks_kinds()
                .validate(&r)
                .unwrap_or_else(|err| panic!("{}: {err}", e.target));
            assert_eq!(e.source, build::wiki_uri("intro"));
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

    #[test]
    fn link_edges_carry_the_lists_own_link_type() {
        let item = build::task_uri("issues", "ENG-10");
        let other = build::task_uri("issues", "ENG-20");
        let edges = task_link_edges(&item, &[(other.clone(), "parent_of".into())]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source, item);
        assert_eq!(edges[0].target, other);
        assert_eq!(edges[0].ref_type, "task_link:parent_of");
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
