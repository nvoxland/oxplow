//! Pure projections from per-kind data into [`PageRefEdge`]s.
//!
//! Each writer that owns a `source_kind` calls one of these helpers
//! to compute the edges its row contributes, then hands the result
//! to [`SqlitePageRefStore::replace_source`]. Keeping the
//! transformation pure (no DB, no IO) makes it trivially unit-
//! testable and lets the boot-time backfill replay the exact same
//! mapping from existing rows.
//!
//! Every stored `(kind, id)` is a canonical ref's `(kind, id)`
//! (`.context/refs.md`), so a row can be turned back into a ref by
//! `format!("{kind}:{id}")` and nothing downstream needs a per-kind id
//! scheme:
//! - wiki:      `"<slug>"`
//! - work_item: `"oxplow:tsk<n>"` — a task, provider-scoped ([`work_item_id`])
//! - file:      `"<repo-relative path>"`
//! - dir:       `"<repo-relative path, no trailing slash>"`
//! - finding:   `"<finding id>"`
//! - commit:    `"<sha>"`
//! - thread_note: `"not<n>"`

use oxplow_domain::refs::kind::KindRegistry;
use oxplow_domain::refs::{extract, RefVersion};
use oxplow_domain::EffortImpact;

use crate::effort_store::FileRefVersion;
use crate::page_ref_store::PageRefEdge;

pub const KIND_WIKI: &str = "wiki";
pub const KIND_WORK_ITEM: &str = "work_item";
pub const KIND_THREAD_NOTE: &str = "thread_note";
pub const KIND_FILE: &str = "file";
pub const KIND_DIR: &str = "dir";
pub const KIND_FINDING: &str = "finding";
pub const KIND_COMMIT: &str = "commit";

// The work-item helpers live with the other ref builders (tsk450).

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
        let is_versioned = edge.target_kind == KIND_FILE || edge.target_kind == KIND_DIR;
        if !is_versioned {
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

/// The kinds a `EffortImpact` may name, as the agent's tools document them.
pub const IMPACT_KINDS: [&str; 6] = [
    "wiki",
    "work_item",
    "file",
    "directory",
    "git_commit",
    "finding",
];

/// The page-ref kind a `EffortImpact.kind` (one of [`IMPACT_KINDS`])
/// projects to; `None` for any other (refused where impacts come in).
pub fn impact_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "wiki" => Some(KIND_WIKI),
        "work_item" => Some(KIND_WORK_ITEM),
        "file" => Some(KIND_FILE),
        "directory" => Some(KIND_DIR),
        "git_commit" => Some(KIND_COMMIT),
        "finding" => Some(KIND_FINDING),
        _ => None,
    }
}

/// Edges contributed by the union of every effort's declared
/// impacts, from the work item `source` (its provider-scoped id,
/// `oxplow:tsk7` or `issues:ENG-12`). Self references are filtered out
/// (an effort on tsk7 declaring it "completed" tsk7 is implicit).
pub fn effort_impact_edges(
    kinds: &KindRegistry,
    source: &str,
    impacts: &[EffortImpact],
) -> Vec<PageRefEdge> {
    let mut out = Vec::new();
    for imp in impacts {
        let Some(target_kind) = impact_kind(&imp.kind) else {
            continue;
        };
        if imp.id.trim().is_empty() {
            continue;
        }
        // A work item is stored canonical however the agent wrote it: its
        // ref, or one of the active list's own ids.
        let target_id = if target_kind == KIND_WORK_ITEM {
            let id = imp.id.trim();
            let Some(target) = id
                .strip_prefix("work_item:")
                .map(str::to_string)
                .or_else(|| kinds.work_item_id(id))
            else {
                continue;
            };
            if target == source {
                continue;
            }
            target
        } else {
            imp.id.clone()
        };
        let mut edge = PageRefEdge::new(KIND_WORK_ITEM, source, target_kind, target_id, RT_IMPACT);
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
pub fn wiki_edges(kinds: &KindRegistry, slug: &str, body: &str) -> Vec<PageRefEdge> {
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        let extra = match fd.version {
            RefVersion::Disk if fd.line.is_none() => None,
            _ => Some(
                serde_json::json!({
                    "line": fd.line,
                    "version": match &fd.version {
                        RefVersion::Disk => "disk".to_string(),
                        RefVersion::Ref(r) => format!("ref:{r}"),
                    }
                })
                .to_string(),
            ),
        };
        let mut edge = PageRefEdge::new(KIND_WIKI, slug, KIND_FILE, fd.path, RT_WIKI_FILE);
        if let Some(e) = extra {
            edge = edge.with_extra(e);
        }
        out.push(edge);
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(KIND_WIKI, slug, KIND_DIR, d, RT_WIKI_DIR));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(KIND_WIKI, slug, KIND_WIKI, w, RT_WIKILINK));
    }
    for t in refs.work_items {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_WORK_ITEM,
            t,
            RT_BODY_WORK_ITEM,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Edges contributed by a task-note body. Single-owner source.
pub fn note_edges(
    kinds: &KindRegistry,
    note_kind: &str,
    note_id: &str,
    body: &str,
) -> Vec<PageRefEdge> {
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_DIR,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for t in refs.work_items {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_WORK_ITEM,
            t,
            RT_BODY_WORK_ITEM,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            note_kind,
            note_id,
            KIND_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// A work item's body-mention edges — what its title and body name — for
/// any list's item; `id` is its `<provider>:<id>`.
pub fn work_item_edges(
    kinds: &KindRegistry,
    id: &str,
    title: &str,
    body: &str,
) -> Vec<PageRefEdge> {
    mention_edges(kinds, id, &format!("{title}\n{body}"), &BODY_MENTIONS)
}

/// A work item's comment-mention edges: what its comments' bodies name.
pub fn work_item_comment_edges<'a>(
    kinds: &KindRegistry,
    id: &str,
    bodies: impl IntoIterator<Item = &'a str>,
) -> Vec<PageRefEdge> {
    let mut out: Vec<PageRefEdge> = Vec::new();
    for body in bodies {
        for edge in mention_edges(kinds, id, body, &COMMENT_MENTIONS) {
            if !out.iter().any(|e| {
                e.target_kind == edge.target_kind
                    && e.target_id == edge.target_id
                    && e.ref_type == edge.ref_type
            }) {
                out.push(edge);
            }
        }
    }
    out
}

/// A work item's link edges: one per `(target ref, link type)`, its ref
/// type `work_item_link:<type>`.
pub fn work_item_link_edges(id: &str, links: &[(String, String)]) -> Vec<PageRefEdge> {
    links
        .iter()
        .filter_map(|(target, link_type)| {
            let target = target.strip_prefix("work_item:")?;
            Some(PageRefEdge::new(
                KIND_WORK_ITEM,
                id,
                KIND_WORK_ITEM,
                target,
                format!("{RT_LINK_PREFIX}{link_type}"),
            ))
        })
        .collect()
}

/// What `text` mentions, as edges from work item `id` under `types`; the
/// item naming itself isn't one.
fn mention_edges(
    kinds: &KindRegistry,
    id: &str,
    text: &str,
    types: &MentionTypes,
) -> Vec<PageRefEdge> {
    let refs = extract(kinds, text);
    let edge = |kind: &str, target: String, ref_type: &str| {
        PageRefEdge::new(KIND_WORK_ITEM, id, kind, target, ref_type)
    };
    let mut out = Vec::new();
    out.extend(
        refs.files_detail
            .into_iter()
            .map(|fd| edge(KIND_FILE, fd.path, types.file)),
    );
    out.extend(refs.dirs.into_iter().map(|d| edge(KIND_DIR, d, types.dir)));
    out.extend(
        refs.wikis
            .into_iter()
            .map(|w| edge(KIND_WIKI, w, types.wiki)),
    );
    out.extend(
        refs.work_items
            .into_iter()
            .filter(|t| t != id)
            .map(|t| edge(KIND_WORK_ITEM, t, types.work_item)),
    );
    out.extend(
        refs.findings
            .into_iter()
            .map(|f| edge(KIND_FINDING, f, types.finding)),
    );
    out.extend(
        refs.commits
            .into_iter()
            .map(|c| edge(KIND_COMMIT, c, types.commit)),
    );
    out
}

/// Touched-file edges for a task.
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
            PageRefEdge::new(
                KIND_WORK_ITEM,
                source,
                KIND_FILE,
                path.clone(),
                RT_TOUCHED_FILE,
            )
            .with_extra(extra)
        })
        .collect()
}

/// Edges contributed by the union of every `effort.summary`
/// body for one work item (`source`, its provider-scoped id). Parsed via
/// the shared ref extractor, so wikilinks (`[[some-slug]]`), file/dir
/// refs, task/finding/commit mentions all flow through as outbound edges
/// from `(work_item, source)`.
/// Owned slice = the `summary_*` ref_types above (paired with
/// `RT_TOUCHED_FILE` under `effort_ref_types()`).
pub fn effort_summary_edges(
    kinds: &KindRegistry,
    source: &str,
    summaries: &[String],
) -> Vec<PageRefEdge> {
    if summaries.is_empty() {
        return Vec::new();
    }
    let combined = summaries.join("\n\n");
    let refs = extract(kinds, &combined);
    let task_id = source;
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_FILE,
            fd.path,
            RT_SUMMARY_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_DIR,
            d,
            RT_SUMMARY_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_WIKI,
            w,
            RT_SUMMARY_WIKILINK,
        ));
    }
    for target in refs.work_items {
        if target == source {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_WORK_ITEM,
            target,
            RT_SUMMARY_WORK_ITEM,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_FINDING,
            f,
            RT_SUMMARY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_COMMIT,
            c,
            RT_SUMMARY_COMMIT,
        ));
    }
    out
}

/// One edge per finding -> file. Owned by the findings writer.
pub fn finding_edges(finding_id: &str, path: &str) -> Vec<PageRefEdge> {
    vec![PageRefEdge::new(
        KIND_FINDING,
        finding_id,
        KIND_FILE,
        path,
        RT_FINDING_PATH,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core kinds with oxplow's tasks as the work list (`tsk<n>`).
    fn tasks_kinds() -> oxplow_domain::refs::kind::KindRegistry {
        oxplow_domain::refs::kind::core_kinds()
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap()
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[tsk7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges(&tasks_kinds(), "intro", body);
        let kinds: std::collections::BTreeSet<_> =
            edges.iter().map(|e| e.target_kind.as_str()).collect();
        assert!(kinds.contains("file"));
        assert!(kinds.contains("dir"));
        assert!(kinds.contains("wiki"));
        assert!(kinds.contains("work_item"));
        assert!(kinds.contains("finding"));
        assert!(kinds.contains("commit"));
        // Every stored (kind, id) is a canonical ref's (kind, id) (tsk404).
        for e in &edges {
            let text = format!("{}:{}", e.target_kind, e.target_id);
            let r = oxplow_domain::refs::grammar::CanonicalRef::parse(&text)
                .unwrap_or_else(|err| panic!("{text}: {err}"));
            tasks_kinds()
                .validate(&r)
                .unwrap_or_else(|err| panic!("{text}: {err}"));
        }
        let task = edges.iter().find(|e| e.target_kind == "work_item").unwrap();
        assert_eq!(task.target_id, "oxplow:tsk7");
    }

    #[test]
    fn work_item_edges_parse_the_body() {
        let edges = work_item_edges(
            &tasks_kinds(),
            "oxplow:tsk1",
            "fix something",
            "see [[src/app.rs]] for context, blocked by tsk2, touches finding:fnd-9",
        );
        let targets: Vec<_> = edges
            .iter()
            .map(|e| (e.target_kind.as_str(), e.target_id.as_str()))
            .collect();
        assert!(targets.contains(&("file", "src/app.rs")));
        assert!(targets.contains(&("work_item", "oxplow:tsk2")));
        assert!(targets.contains(&("finding", "fnd-9")));
        // self-mention filtered out
        assert!(!targets
            .iter()
            .any(|(k, id)| *k == "work_item" && *id == "oxplow:tsk1"));
        // The source is the task's own canonical (kind, id).
        assert!(edges
            .iter()
            .all(|e| e.source_kind == "work_item" && e.source_id == "oxplow:tsk1"));
    }

    #[test]
    fn effort_touched_file_edges_one_per_path() {
        let entries = vec![
            ("a.rs".to_string(), "created".to_string()),
            ("b.rs".to_string(), "updated".to_string()),
            ("c.rs".to_string(), "deleted".to_string()),
        ];
        let edges = effort_touched_file_edges("oxplow:tsk7", &entries);
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[0].source_kind, "work_item");
        assert_eq!(edges[0].source_id, "oxplow:tsk7");
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
        let edges = effort_summary_edges(&tasks_kinds(), "oxplow:tsk7", &summaries);
        let by_kind: std::collections::BTreeMap<_, Vec<_>> =
            edges
                .iter()
                .fold(std::collections::BTreeMap::new(), |mut m, e| {
                    m.entry(e.target_kind.as_str())
                        .or_default()
                        .push((e.target_id.as_str(), e.ref_type.as_str()));
                    m
                });
        assert!(by_kind.get("wiki").is_some_and(|v| v
            .iter()
            .any(|(id, rt)| *id == "url-schemes" && *rt == "summary_wikilink")));
        assert!(by_kind.get("file").is_some_and(|v| v
            .iter()
            .any(|(id, rt)| *id == "src/foo.rs" && *rt == "summary_file_ref")));
        assert!(by_kind.get("work_item").is_some_and(|v| v
            .iter()
            .any(|(id, rt)| *id == "oxplow:tsk99" && *rt == "summary_work_item_mention")));
        assert!(by_kind.get("finding").is_some_and(|v| v
            .iter()
            .any(|(id, rt)| *id == "fnd-2" && *rt == "summary_finding_mention")));
        assert!(by_kind.get("commit").is_some_and(|v| !v.is_empty()));
        assert!(by_kind.get("dir").is_some_and(|v| v
            .iter()
            .any(|(id, rt)| *id == "src/x" && *rt == "summary_dir_ref")));
    }

    #[test]
    fn effort_summary_edges_filter_self_task() {
        let summaries = vec!["wraps up tsk7 itself and references tsk9".into()];
        let edges = effort_summary_edges(&tasks_kinds(), "oxplow:tsk7", &summaries);
        let task_ids: Vec<_> = edges
            .iter()
            .filter(|e| e.target_kind == "work_item")
            .map(|e| e.target_id.as_str())
            .collect();
        assert_eq!(task_ids, vec!["oxplow:tsk9"]);
    }

    #[test]
    fn effort_summary_edges_empty_input_yields_no_edges() {
        assert!(effort_summary_edges(&tasks_kinds(), "oxplow:tsk7", &[]).is_empty());
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
        let edges = effort_impact_edges(&tasks_kinds(), "oxplow:tsk7", &impacts);
        assert_eq!(edges.len(), 3, "got {edges:?}");
        let wiki = edges
            .iter()
            .find(|e| e.target_kind == "wiki")
            .expect("wiki edge");
        assert_eq!(wiki.target_id, "url-schemes");
        assert!(wiki
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));
        let commit = edges
            .iter()
            .find(|e| e.target_kind == "commit")
            .expect("commit edge");
        assert_eq!(commit.target_id, "abc1234");
        let dir = edges
            .iter()
            .find(|e| e.target_kind == "dir")
            .expect("dir edge");
        assert_eq!(dir.target_id, "src/x");
        assert!(dir.source_extra.is_none());
    }

    /// A work-item impact names the active list's item: a loose id as the
    /// list declares its ids, or its ref; stored canonical either way.
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
            impact("work_item:oxplow:tsk11"),
            impact("11"),
            impact("ENG-12"),
        ];
        let ids: Vec<String> = effort_impact_edges(&tasks_kinds(), "oxplow:tsk7", &impacts)
            .into_iter()
            .map(|e| e.target_id)
            .collect();
        assert_eq!(ids, vec!["oxplow:tsk9", "oxplow:tsk11"]);
    }

    #[test]
    fn link_edges_carry_the_lists_own_link_type() {
        let edges = work_item_link_edges(
            "issues:ENG-10",
            &[("work_item:issues:ENG-20".into(), "parent_of".into())],
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_kind, "work_item");
        assert_eq!(edges[0].source_id, "issues:ENG-10");
        assert_eq!(edges[0].target_id, "issues:ENG-20");
        assert_eq!(edges[0].ref_type, "work_item_link:parent_of");
    }

    /// Comments' mentions are the item's own edges, under the comment
    /// ref types, each target once.
    #[test]
    fn comment_edges_are_the_items_under_comment_types() {
        let edges = work_item_comment_edges(
            &tasks_kinds(),
            "oxplow:tsk1",
            ["see [[src/app.rs]] and tsk2", "again [[src/app.rs]]"],
        );
        let got: Vec<(&str, &str, &str)> = edges
            .iter()
            .map(|e| {
                (
                    e.source_id.as_str(),
                    e.target_id.as_str(),
                    e.ref_type.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("oxplow:tsk1", "src/app.rs", RT_COMMENT_FILE),
                ("oxplow:tsk1", "oxplow:tsk2", RT_COMMENT_WORK_ITEM),
            ]
        );
    }

    #[test]
    fn finding_edges_point_at_file() {
        let edges = finding_edges("fnd-7", "src/app.rs");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_kind, "finding");
        assert_eq!(edges[0].target_id, "src/app.rs");
    }
}
