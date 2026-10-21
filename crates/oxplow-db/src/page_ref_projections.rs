//! Pure projections from per-source data into [`PageRefEdge`]s.
//!
//! Each writer that owns a source calls one of these helpers to compute
//! the edges its row contributes, then hands the result to
//! [`SqlitePageRefStore::replace_source`]. Keeping the transformation
//! pure (no DB, no IO) makes it trivially unit-testable and lets the
//! boot-time backfill replay the exact same mapping from existing rows.
//!
//! Every end is a URI (`.context/resources.md`), built here through
//! `uri::build`: a wiki page's, a work item's, a file's, a directory's, a
//! finding's, a commit's, a thread note's.

use oxplow_domain::uri::build;
use oxplow_domain::uri::resources::{self as r, ResourceRegistry};
use oxplow_domain::uri::{extract, RefVersion};
use oxplow_domain::EffortImpact;

use crate::effort_store::FileRefVersion;
use crate::page_ref_store::PageRefEdge;

pub const RT_WIKI_FILE: &str = "wiki_file_ref";
pub const RT_WIKI_DIR: &str = "wiki_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
pub const RT_BODY_WORK_ITEM: &str = "work_item_mention";
pub const RT_BODY_FINDING: &str = "finding_mention";
pub const RT_BODY_COMMIT: &str = "commit_mention";
pub const RT_TOUCHED_FILE: &str = "touched_file";
pub const RT_FINDING_PATH: &str = "finding_path";

// Ref-types written by the effort store from union of all
// `effort.summary` bodies for a task. Distinct from
// `task_body_*` so the body slice (task_store) and the summary
// slice (effort_store) can coexist under the same `(task, id)`
// source without clobbering each other.
pub const RT_SUMMARY_FILE: &str = "summary_file_ref";

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
pub const RT_SUMMARY_DIR: &str = "summary_dir_ref";
pub const RT_SUMMARY_WIKILINK: &str = "summary_wikilink";
pub const RT_SUMMARY_WORK_ITEM: &str = "summary_work_item_mention";
pub const RT_SUMMARY_FINDING: &str = "summary_finding_mention";
pub const RT_SUMMARY_COMMIT: &str = "summary_commit_mention";

/// Declared impacts (per-effort `EffortImpact` rows) — the action
/// taken is carried in `source_extra` as `{"action": "..."}`.
/// Single ref_type covers every impacted kind because the target
/// kind already discriminates wiki vs task vs file vs etc.
pub const RT_IMPACT: &str = "impact";

/// A work item → the commit that holds its effort's work: the commit's
/// version of every file they share is the effort's end version (tsk1035,
/// `commit_links`). The effort is carried in `source_extra` as
/// `{"effort": "eff12"}`.
pub const RT_COMMITTED: &str = "committed";

/// A work item's body slice: its title and body's mentions.
pub fn work_item_body_ref_types() -> Vec<String> {
    BODY_MENTIONS.all()
}

/// A work item's comment slice: its comments' mentions, recorded as the
/// item's own edges (a comment lives on its item's page).
pub fn work_item_comment_ref_types() -> Vec<String> {
    COMMENT_MENTIONS.all()
}

/// A work item's link slice is every ref type with this prefix — one per
/// link type, the list's own (`work_item_link:blocks`).
pub const RT_LINK_PREFIX: &str = "work_item_link:";

pub const RT_COMMENT_FILE: &str = "comment_file_ref";
pub const RT_COMMENT_DIR: &str = "comment_dir_ref";
pub const RT_COMMENT_WIKILINK: &str = "comment_wikilink";
pub const RT_COMMENT_WORK_ITEM: &str = "comment_work_item_mention";
pub const RT_COMMENT_FINDING: &str = "comment_finding_mention";
pub const RT_COMMENT_COMMIT: &str = "comment_commit_mention";

/// The ref types a text's mentions are recorded under, by what they name.
struct MentionTypes {
    file: &'static str,
    dir: &'static str,
    wiki: &'static str,
    work_item: &'static str,
    finding: &'static str,
    commit: &'static str,
}

impl MentionTypes {
    fn all(&self) -> Vec<String> {
        [
            self.file,
            self.dir,
            self.wiki,
            self.work_item,
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
    work_item: RT_BODY_WORK_ITEM,
    finding: RT_BODY_FINDING,
    commit: RT_BODY_COMMIT,
};

const COMMENT_MENTIONS: MentionTypes = MentionTypes {
    file: RT_COMMENT_FILE,
    dir: RT_COMMENT_DIR,
    wiki: RT_COMMENT_WIKILINK,
    work_item: RT_COMMENT_WORK_ITEM,
    finding: RT_COMMENT_FINDING,
    commit: RT_COMMENT_COMMIT,
};

/// Slice owned by the effort store: the union of touched-file
/// edges across every effort on a task, the projection of every
/// `effort.summary` body parsed for refs, and the declared
/// `EffortImpact` rows for each effort.
pub fn effort_ref_types() -> Vec<String> {
    vec![
        RT_TOUCHED_FILE.to_string(),
        RT_SUMMARY_FILE.to_string(),
        RT_SUMMARY_DIR.to_string(),
        RT_SUMMARY_WIKILINK.to_string(),
        RT_SUMMARY_WORK_ITEM.to_string(),
        RT_SUMMARY_FINDING.to_string(),
        RT_SUMMARY_COMMIT.to_string(),
        RT_IMPACT.to_string(),
    ]
}

const SUMMARY_MENTIONS: MentionTypes = MentionTypes {
    file: RT_SUMMARY_FILE,
    dir: RT_SUMMARY_DIR,
    wiki: RT_SUMMARY_WIKILINK,
    work_item: RT_SUMMARY_WORK_ITEM,
    finding: RT_SUMMARY_FINDING,
    commit: RT_SUMMARY_COMMIT,
};

/// The kinds a `EffortImpact` may name, as the agent's tools document them.
pub const IMPACT_KINDS: [&str; 6] = [
    "wiki",
    "work_item",
    "file",
    "directory",
    "git_commit",
    "finding",
];

/// The URI an `EffortImpact` names: its `kind` (one of [`IMPACT_KINDS`])
/// and `id`. A work item's id is its URI or one of the active list's own
/// ids (`tsk7`); `None` for anything else (refused where impacts come in).
pub fn impact_target(kinds: &ResourceRegistry, kind: &str, id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    Some(match kind {
        "wiki" => build::wiki_uri(id),
        "work_item" => match build::work_item_of_uri(id) {
            Some((provider, item)) => build::work_item_uri(&provider, &item),
            None => kinds.work_item_uri_of(id)?,
        },
        "file" => build::file_uri(id),
        "directory" => build::dir_uri(id),
        "git_commit" => build::commit_uri(id),
        "finding" => build::finding_uri(id),
        _ => return None,
    })
}

/// Edges contributed by the union of every effort's declared impacts,
/// from the work item `source` (its URI). Self references are filtered
/// out (an effort on tsk7 declaring it "completed" tsk7 is implicit).
pub fn effort_impact_edges(
    kinds: &ResourceRegistry,
    source: &str,
    impacts: &[EffortImpact],
) -> Vec<PageRefEdge> {
    let mut out = Vec::new();
    for imp in impacts {
        let Some(target) = impact_target(kinds, &imp.kind, &imp.id) else {
            continue;
        };
        if target == source {
            continue;
        }
        let mut edge = PageRefEdge::new(source, target, RT_IMPACT);
        if let Some(action) = &imp.action {
            if !action.trim().is_empty() {
                edge = edge.with_extra(serde_json::json!({ "action": action.trim() }).to_string());
            }
        }
        out.push(edge);
    }
    out
}

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
        r::WORK_ITEM => RT_BODY_WORK_ITEM,
        r::FINDING => RT_BODY_FINDING,
        r::COMMIT => RT_BODY_COMMIT,
        _ => return None,
    })
}

/// Edges of a wiki page whose store states its links (a provider's
/// record: a file's URI, another page's, a work item's, …) rather than a
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

/// A work item's body-mention edges — what its title and body name — for
/// any list's item; `item` is its URI.
pub fn work_item_edges(
    kinds: &ResourceRegistry,
    item: &str,
    title: &str,
    body: &str,
) -> Vec<PageRefEdge> {
    mention_edges(kinds, item, &format!("{title}\n{body}"), &BODY_MENTIONS)
}

/// A work item's comment-mention edges: what its comments' bodies name.
pub fn work_item_comment_edges<'a>(
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

/// A work item's link edges: one per `(target URI, link type)`, its ref
/// type `work_item_link:<type>`. Any URI: another item, or what the item
/// is about.
pub fn work_item_link_edges(item: &str, links: &[(String, String)]) -> Vec<PageRefEdge> {
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
        refs.work_items
            .into_iter()
            .filter(|t| t != source)
            .map(|t| edge(t, types.work_item)),
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

/// Touched-file edges for a work item (`source`, its URI).
///
/// `entries` is `(path, change_kind)` — the change_kind is one of
/// the `effort_file.change_kind` values (`created` / `updated`
/// / `deleted`) and is carried through `source_extra` as
/// `{"change_kind":"..."}` so the renderer can display "created"
/// / "modified" / "deleted" instead of a single "touched" label.
/// The renderer normalizes `updated` → "modified" for display.
pub fn effort_touched_file_edges(source: &str, entries: &[(String, String)]) -> Vec<PageRefEdge> {
    entries
        .iter()
        .map(|(path, change_kind)| {
            let extra = serde_json::json!({ "change_kind": change_kind }).to_string();
            PageRefEdge::new(source, build::file_uri(path), RT_TOUCHED_FILE).with_extra(extra)
        })
        .collect()
}

/// Edges contributed by the union of every `effort.summary` body for one
/// work item (`source`, its URI). Parsed via the shared ref extractor, so
/// wikilinks (`[[some-slug]]`), file/dir refs, task/finding/commit
/// mentions all flow through as outbound edges from the item.
/// Owned slice = the `summary_*` ref_types above (paired with
/// `RT_TOUCHED_FILE` under `effort_ref_types()`).
pub fn effort_summary_edges(
    kinds: &ResourceRegistry,
    source: &str,
    summaries: &[String],
) -> Vec<PageRefEdge> {
    if summaries.is_empty() {
        return Vec::new();
    }
    mention_edges(kinds, source, &summaries.join("\n\n"), &SUMMARY_MENTIONS)
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

    /// Foundation's resources with oxplow's tasks as the work list (`tsk<n>`).
    fn tasks_kinds() -> oxplow_domain::uri::resources::ResourceRegistry {
        oxplow_domain::uri::resources::foundation()
            .clone()
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap()
    }

    fn task(id: &str) -> String {
        build::work_item_uri("oxplow", id)
    }

    fn resource(e: &PageRefEdge) -> String {
        resource_of(&e.target).unwrap()
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[tsk7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges(&tasks_kinds(), "intro", body);
        let kinds: std::collections::BTreeSet<_> = edges.iter().map(resource).collect();
        for k in ["file", "dir", "wiki", "work_item", "finding", "commit"] {
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
    fn work_item_edges_parse_the_body() {
        let edges = work_item_edges(
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
    fn effort_touched_file_edges_one_per_path() {
        let entries = vec![
            ("a.rs".to_string(), "created".to_string()),
            ("b.rs".to_string(), "updated".to_string()),
            ("c.rs".to_string(), "deleted".to_string()),
        ];
        let edges = effort_touched_file_edges(&task("tsk7"), &entries);
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[0].source, task("tsk7"));
        assert_eq!(edges[0].target, build::file_uri("a.rs"));
        assert_eq!(edges[0].ref_type, "touched_file");
        assert!(edges[0]
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));
        assert!(edges[1]
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("updated")));
        assert!(edges[2]
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("deleted")));
    }

    #[test]
    fn effort_summary_edges_extract_all_kinds() {
        let summaries = vec![
            "Filed [[url-schemes]] with refs to [[src/foo.rs]]".to_string(),
            "Resolved tsk99 and finding:fnd-2; see [[git:abcdef0]] and [[dir:src/x]]".to_string(),
        ];
        let edges = effort_summary_edges(&tasks_kinds(), &task("tsk7"), &summaries);
        let has =
            |target: String, rt: &str| edges.iter().any(|e| e.target == target && e.ref_type == rt);
        assert!(has(build::wiki_uri("url-schemes"), "summary_wikilink"));
        assert!(has(build::file_uri("src/foo.rs"), "summary_file_ref"));
        assert!(has(task("tsk99"), "summary_work_item_mention"));
        assert!(has(build::finding_uri("fnd-2"), "summary_finding_mention"));
        assert!(edges.iter().any(|e| resource(e) == "oxplow/commit"));
        assert!(has(build::dir_uri("src/x"), "summary_dir_ref"));
    }

    #[test]
    fn effort_summary_edges_filter_self_task() {
        let summaries = vec!["wraps up tsk7 itself and references tsk9".into()];
        let edges = effort_summary_edges(&tasks_kinds(), &task("tsk7"), &summaries);
        let tasks: Vec<_> = edges
            .iter()
            .filter(|e| resource(e) == "oxplow/work_item")
            .map(|e| e.target.clone())
            .collect();
        assert_eq!(tasks, vec![task("tsk9")]);
    }

    #[test]
    fn effort_summary_edges_empty_input_yields_no_edges() {
        assert!(effort_summary_edges(&tasks_kinds(), &task("tsk7"), &[]).is_empty());
    }

    #[test]
    fn effort_impact_edges_normalize_kinds_and_carry_action() {
        use oxplow_domain::EffortImpact;
        let impacts = vec![
            EffortImpact {
                kind: "wiki".into(),
                id: "url-schemes".into(),
                action: Some("created".into()),
            },
            EffortImpact {
                kind: "git_commit".into(),
                id: "abc1234".into(),
                action: Some("referenced".into()),
            },
            EffortImpact {
                kind: "directory".into(),
                id: "src/x".into(),
                action: None,
            },
            EffortImpact {
                kind: "dir".into(),
                id: "src/y".into(),
                action: None,
            }, // not an impact kind — filtered
            EffortImpact {
                kind: "work_item".into(),
                id: "tsk7".into(),
                action: Some("completed".into()),
            }, // self — filtered
            EffortImpact {
                kind: "bogus".into(),
                id: "x".into(),
                action: None,
            }, // bad kind — filtered
            EffortImpact {
                kind: "work_item".into(),
                id: "".into(),
                action: None,
            }, // empty id — filtered
        ];
        let edges = effort_impact_edges(&tasks_kinds(), &task("tsk7"), &impacts);
        assert_eq!(edges.len(), 3, "got {edges:?}");
        let wiki = edges
            .iter()
            .find(|e| e.target == build::wiki_uri("url-schemes"))
            .expect("wiki edge");
        assert!(wiki
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));
        assert!(edges
            .iter()
            .any(|e| e.target == build::commit_uri("abc1234")));
        let dir = edges
            .iter()
            .find(|e| e.target == build::dir_uri("src/x"))
            .expect("dir edge");
        assert!(dir.source_extra.is_none());
    }

    /// A work-item impact names the active list's item: a loose id as the
    /// list declares its ids, or its URI; stored canonical either way.
    /// What isn't one of its ids names nothing.
    #[test]
    fn impact_work_items_are_stored_canonical_whatever_the_agent_wrote() {
        use oxplow_domain::EffortImpact;
        let impact = |id: &str| EffortImpact {
            kind: "work_item".into(),
            id: id.into(),
            action: None,
        };
        let impacts = vec![
            impact("tsk9"),
            impact(&task("tsk11")),
            impact("11"),
            impact("ENG-12"),
        ];
        let targets: Vec<String> = effort_impact_edges(&tasks_kinds(), &task("tsk7"), &impacts)
            .into_iter()
            .map(|e| e.target)
            .collect();
        assert_eq!(targets, vec![task("tsk9"), task("tsk11")]);
    }

    #[test]
    fn link_edges_carry_the_lists_own_link_type() {
        let item = build::work_item_uri("issues", "ENG-10");
        let other = build::work_item_uri("issues", "ENG-20");
        let edges = work_item_link_edges(&item, &[(other.clone(), "parent_of".into())]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source, item);
        assert_eq!(edges[0].target, other);
        assert_eq!(edges[0].ref_type, "work_item_link:parent_of");
    }

    /// Comments' mentions are the item's own edges, under the comment
    /// ref types, each target once.
    #[test]
    fn comment_edges_are_the_items_under_comment_types() {
        let edges = work_item_comment_edges(
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
                (task("tsk1"), task("tsk2"), RT_COMMENT_WORK_ITEM),
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
