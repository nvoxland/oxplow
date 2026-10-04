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
//! - task_note: `"not<n>"`

use oxplow_domain::refs::kind::KindRegistry;
use oxplow_domain::refs::{extract, RefVersion};
use oxplow_domain::{Task, TaskId, TaskImpact, TaskLink, TaskLinkType};

use crate::effort_store::FileRefVersion;
use crate::page_ref_store::PageRefEdge;

pub const KIND_WIKI: &str = "wiki";
pub const KIND_WORK_ITEM: &str = "work_item";
pub const KIND_TASK_NOTE: &str = "task_note";
pub const KIND_FILE: &str = "file";
pub const KIND_DIR: &str = "dir";
pub const KIND_FINDING: &str = "finding";
pub const KIND_COMMIT: &str = "commit";

// The work-item helpers live with the other ref builders (tsk450).
pub use oxplow_domain::refs::build::{task_from_work_item_id, work_item_id, OXPLOW_PROVIDER};

pub const RT_WIKI_FILE: &str = "wiki_file_ref";
pub const RT_WIKI_DIR: &str = "wiki_dir_ref";
pub const RT_WIKILINK: &str = "wikilink";
pub const RT_BODY_TASK: &str = "task_body_mention";
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
pub const RT_SUMMARY_TASK: &str = "summary_task_mention";
pub const RT_SUMMARY_FINDING: &str = "summary_finding_mention";
pub const RT_SUMMARY_COMMIT: &str = "summary_commit_mention";

/// Declared impacts (per-effort `TaskImpact` rows) — the action
/// taken is carried in `source_extra` as `{"action": "..."}`.
/// Single ref_type covers every impacted kind because the target
/// kind already discriminates wiki vs task vs file vs etc.
pub const RT_IMPACT: &str = "impact";

/// Ref-types written by the task store from a body (title +
/// description + AC). Used by the slice-replace call so other
/// writers' rows for the same `task:<id>` source survive.
pub fn task_body_ref_types() -> Vec<String> {
    vec![
        RT_WIKI_FILE.to_string(),
        RT_WIKI_DIR.to_string(),
        RT_WIKILINK.to_string(),
        RT_BODY_TASK.to_string(),
        RT_BODY_FINDING.to_string(),
        RT_BODY_COMMIT.to_string(),
    ]
}

/// All link sub-types — the link store owns this slice for any
/// given source task.
pub fn task_link_ref_types() -> Vec<String> {
    [
        "blocks",
        "relates_to",
        "discovered_from",
        "duplicates",
        "supersedes",
        "replies_to",
    ]
    .iter()
    .map(|t| format!("task_link:{t}"))
    .collect()
}

/// Slice owned by the effort store: the union of touched-file
/// edges across every effort on a task, the projection of every
/// `effort.summary` body parsed for refs, and the declared
/// `TaskImpact` rows for each effort.
pub fn effort_ref_types() -> Vec<String> {
    vec![
        RT_TOUCHED_FILE.to_string(),
        RT_SUMMARY_FILE.to_string(),
        RT_SUMMARY_DIR.to_string(),
        RT_SUMMARY_WIKILINK.to_string(),
        RT_SUMMARY_TASK.to_string(),
        RT_SUMMARY_FINDING.to_string(),
        RT_SUMMARY_COMMIT.to_string(),
        RT_IMPACT.to_string(),
    ]
}

/// The kinds a `TaskImpact` may name, as the agent's tools document them.
pub const IMPACT_KINDS: [&str; 6] = ["wiki", "task", "file", "directory", "git_commit", "finding"];

/// The page-ref kind a `TaskImpact.kind` (one of [`IMPACT_KINDS`])
/// projects to; `None` for any other (refused where impacts come in).
pub fn impact_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "wiki" => Some(KIND_WIKI),
        "task" => Some(KIND_WORK_ITEM),
        "file" => Some(KIND_FILE),
        "directory" => Some(KIND_DIR),
        "git_commit" => Some(KIND_COMMIT),
        "finding" => Some(KIND_FINDING),
        _ => None,
    }
}

/// Edges contributed by the union of every effort's declared
/// impacts, from the work item `source` (its provider-scoped id,
/// `oxplow:tsk7` or `linear:ENG-12`). Self references are filtered out
/// (an effort on tsk7 declaring it "completed" tsk7 is implicit).
pub fn effort_impact_edges(source: &str, impacts: &[TaskImpact]) -> Vec<PageRefEdge> {
    let mut out = Vec::new();
    for imp in impacts {
        let Some(target_kind) = impact_kind(&imp.kind) else {
            continue;
        };
        if imp.id.trim().is_empty() {
            continue;
        }
        // A task target is stored canonical however the agent wrote it.
        let target_id = if target_kind == KIND_WORK_ITEM {
            let Some(t) = task_from_work_item_id(imp.id.trim()) else {
                continue;
            };
            let target = work_item_id(t);
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
    for t in refs.tasks {
        out.push(PageRefEdge::new(
            KIND_WIKI,
            slug,
            KIND_WORK_ITEM,
            work_item_id(TaskId::new(t)),
            RT_BODY_TASK,
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
pub fn note_edges(kinds: &KindRegistry, note_id: &str, body: &str) -> Vec<PageRefEdge> {
    let refs = extract(kinds, body);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_DIR,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for t in refs.tasks {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_WORK_ITEM,
            work_item_id(TaskId::new(t)),
            RT_BODY_TASK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_TASK_NOTE,
            note_id,
            KIND_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Edges contributed by a task's title + description text.
pub fn task_edges(kinds: &KindRegistry, item: &Task) -> Vec<PageRefEdge> {
    let mut combined = String::new();
    combined.push_str(&item.title);
    combined.push('\n');
    combined.push_str(&item.description);
    let refs = extract(kinds, &combined);
    let id = work_item_id(item.id);
    let mut out = Vec::new();
    for fd in refs.files_detail {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_FILE,
            fd.path,
            RT_WIKI_FILE,
        ));
    }
    for d in refs.dirs {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_DIR,
            d,
            RT_WIKI_DIR,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for t in refs.tasks {
        if t == item.id.value() {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_WORK_ITEM,
            work_item_id(TaskId::new(t)),
            RT_BODY_TASK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            &id,
            KIND_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
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
    for t in refs.tasks {
        let target = work_item_id(TaskId::new(t));
        if target == source {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_WORK_ITEM,
            task_id,
            KIND_WORK_ITEM,
            target,
            RT_SUMMARY_TASK,
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

fn link_type_str(t: TaskLinkType) -> &'static str {
    match t {
        TaskLinkType::Blocks => "blocks",
        TaskLinkType::RelatesTo => "relates_to",
        TaskLinkType::DiscoveredFrom => "discovered_from",
        TaskLinkType::Duplicates => "duplicates",
        TaskLinkType::Supersedes => "supersedes",
        TaskLinkType::RepliesTo => "replies_to",
    }
}

/// One edge per `TaskLink`. Source is `from_item`, target is
/// `to_item`, ref_type encodes the link sub-type.
pub fn link_edge(link: &TaskLink) -> PageRefEdge {
    PageRefEdge::new(
        KIND_WORK_ITEM,
        work_item_id(link.from_item_id),
        KIND_WORK_ITEM,
        work_item_id(link.to_item_id),
        format!("task_link:{}", link_type_str(link.link_type)),
    )
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
    use oxplow_domain::refs::kind::core_kinds;
    use oxplow_domain::{
        Task, TaskActorKind, TaskAuthor, TaskId, TaskLinkType, TaskPriority, TaskStatus, Timestamp,
    };

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn item(id: i64, title: &str, description: &str) -> Task {
        Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: title.into(),
            description: description.into(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: ts(),
            updated_at: ts(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        }
    }

    #[test]
    fn wiki_edges_cover_all_kinds() {
        let body = "[[src/app.rs]] [[dir:src]] [[architecture]] [[tsk7]] [[finding:fnd-1]] [[git:abcdef0]]";
        let edges = wiki_edges(&core_kinds(), "intro", body);
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
            oxplow_domain::refs::kind::core_kinds()
                .validate(&r)
                .unwrap_or_else(|err| panic!("{text}: {err}"));
        }
        let task = edges.iter().find(|e| e.target_kind == "work_item").unwrap();
        assert_eq!(task.target_id, "oxplow:tsk7");
    }

    #[test]
    fn task_edges_parse_description() {
        let it = item(
            1,
            "fix something",
            "see [[src/app.rs]] for context, blocked by tsk2, touches finding:fnd-9",
        );
        let edges = task_edges(&core_kinds(), &it);
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
        let edges = effort_summary_edges(&core_kinds(), "oxplow:tsk7", &summaries);
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
            .any(|(id, rt)| *id == "oxplow:tsk99" && *rt == "summary_task_mention")));
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
        let edges = effort_summary_edges(&core_kinds(), "oxplow:tsk7", &summaries);
        let task_ids: Vec<_> = edges
            .iter()
            .filter(|e| e.target_kind == "work_item")
            .map(|e| e.target_id.as_str())
            .collect();
        assert_eq!(task_ids, vec!["oxplow:tsk9"]);
    }

    #[test]
    fn effort_summary_edges_empty_input_yields_no_edges() {
        assert!(effort_summary_edges(&core_kinds(), "oxplow:tsk7", &[]).is_empty());
    }

    #[test]
    fn effort_impact_edges_normalize_kinds_and_carry_action() {
        use oxplow_domain::TaskImpact;
        let impacts = vec![
            TaskImpact {
                kind: "wiki".into(),
                id: "url-schemes".into(),
                action: Some("created".into()),
            },
            TaskImpact {
                kind: "git_commit".into(),
                id: "abc1234".into(),
                action: Some("referenced".into()),
            },
            TaskImpact {
                kind: "directory".into(),
                id: "src/x".into(),
                action: None,
            },
            TaskImpact {
                kind: "dir".into(),
                id: "src/y".into(),
                action: None,
            }, // not an impact kind — filtered
            TaskImpact {
                kind: "task".into(),
                id: "7".into(),
                action: Some("completed".into()),
            }, // self — filtered
            TaskImpact {
                kind: "bogus".into(),
                id: "x".into(),
                action: None,
            }, // bad kind — filtered
            TaskImpact {
                kind: "task".into(),
                id: "".into(),
                action: None,
            }, // empty id — filtered
        ];
        let edges = effort_impact_edges("oxplow:tsk7", &impacts);
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

    #[test]
    fn impact_task_ids_are_stored_canonical_whatever_the_agent_wrote() {
        use oxplow_domain::TaskImpact;
        let impacts = vec![
            TaskImpact {
                kind: "task".into(),
                id: "tsk9".into(),
                action: None,
            },
            TaskImpact {
                kind: "task".into(),
                id: "11".into(),
                action: None,
            },
        ];
        let ids: Vec<String> = effort_impact_edges("oxplow:tsk7", &impacts)
            .into_iter()
            .map(|e| e.target_id)
            .collect();
        assert_eq!(ids, vec!["oxplow:tsk9", "oxplow:tsk11"]);
    }

    #[test]
    fn link_edge_labels_link_subtype() {
        use oxplow_domain::TaskLinkId;
        let link = TaskLink {
            id: TaskLinkId::new(1),
            thread_id: oxplow_domain::ThreadId::new(1),
            from_item_id: TaskId::new(10),
            to_item_id: TaskId::new(20),
            link_type: TaskLinkType::Blocks,
            created_at: ts(),
        };
        let edge = link_edge(&link);
        assert_eq!(edge.source_kind, "work_item");
        assert_eq!(edge.source_id, "oxplow:tsk10");
        assert_eq!(edge.target_id, "oxplow:tsk20");
        assert_eq!(edge.ref_type, "task_link:blocks");
    }

    #[test]
    fn finding_edges_point_at_file() {
        let edges = finding_edges("fnd-7", "src/app.rs");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_kind, "finding");
        assert_eq!(edges[0].target_id, "src/app.rs");
    }
}
