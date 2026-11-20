//! Per-function churn: line-diff base vs head content and attribute
//! added / deleted line counts to the function each line lands in.
//!
//! The Change Analysis dashboard already knows file-level
//! `additions` / `deletions` and function-level
//! `complexityDelta` / `lengthDelta`, but not "function `foo` had
//! 8 lines added and 3 deleted." This module fills that gap.
//!
//! Implementation: feed both side contents through
//! `similar::TextDiff::from_lines` and translate the resulting
//! Insert / Delete ops into 1-indexed line numbers in the new /
//! old sides respectively. Then attribute each added line to the
//! head-side function whose `[start_line, end_line]` interval
//! contains it; each deleted line to the base-side function, then
//! map to the head-side row by symbol id (`Parser::parse(String)`),
//! so overloads and same-named functions stay apart.
//!
//! `modified_lines = min(added, deleted)` per function — cheap,
//! explainable, and good enough as a "this function was edited
//! both ways" signal for ranking. Exact line-by-line alignment
//! would require a Myers diff per function and isn't worth the
//! cost.

use oxplow_code_metrics::FunctionMetrics;
use similar::{ChangeTag, TextDiff};

/// Per-function churn for one file.
#[derive(Debug, Clone)]
pub struct FunctionChurn {
    /// The function's symbol id, which matches base to head.
    pub id: String,
    pub name: String,
    pub container_path: Vec<String>,
    pub start_line_head: u32,
    pub added_lines: u32,
    pub deleted_lines: u32,
    pub modified_lines: u32,
}

/// File-level churn rollup with per-function breakdown.
#[derive(Debug, Clone)]
pub struct FileChurn {
    pub path: String,
    pub file_added: u32,
    pub file_deleted: u32,
    pub functions: Vec<FunctionChurn>,
}

/// Build per-function churn for one file given the base and head
/// content. Either side may be empty (added file → empty base;
/// deleted file → empty head).
///
/// `base_metrics` and `head_metrics` are the function lists already
/// produced by `analyze_file` on each side; we re-use them for
/// interval lookup instead of re-parsing.
pub fn compute_file_churn(
    path: &str,
    base_metrics: &[FunctionMetrics],
    head_metrics: &[FunctionMetrics],
    base_content: &str,
    head_content: &str,
) -> FileChurn {
    let (added_in_head, deleted_in_base) = diff_line_numbers(base_content, head_content);
    let file_added = added_in_head.len() as u32;
    let file_deleted = deleted_in_base.len() as u32;

    if file_added == 0 && file_deleted == 0 {
        return FileChurn {
            path: path.to_string(),
            file_added,
            file_deleted,
            functions: Vec::new(),
        };
    }

    // Per-head-function counters keyed by symbol id. Vec keeps
    // deterministic ordering and avoids pulling in a hash dep just
    // for this.
    let mut counters: Vec<(String, FunctionChurn)> = head_metrics
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                FunctionChurn {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    container_path: m.container_path.clone(),
                    start_line_head: m.start_line,
                    added_lines: 0,
                    deleted_lines: 0,
                    modified_lines: 0,
                },
            )
        })
        .collect();

    // Sort head intervals by start_line for cheap walk-attribution.
    let mut head_intervals: Vec<(u32, u32, usize)> = head_metrics
        .iter()
        .enumerate()
        .map(|(i, m)| (m.start_line, m.end_line, i))
        .collect();
    head_intervals.sort_by_key(|t| t.0);

    for line_no in added_in_head {
        if let Some(idx) = locate_interval(&head_intervals, line_no) {
            counters[idx].1.added_lines += 1;
        }
    }

    // Owned String keys so the immutable borrow on `counters` ends
    // before the deletion loop starts mutating them.
    let head_qkey_to_idx: std::collections::HashMap<String, usize> = counters
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();

    let mut base_intervals: Vec<(u32, u32, String)> = base_metrics
        .iter()
        .map(|m| (m.start_line, m.end_line, m.id.clone()))
        .collect();
    base_intervals.sort_by_key(|t| t.0);

    for line_no in deleted_in_base {
        if let Some(qkey) = locate_qkey(&base_intervals, line_no) {
            if let Some(&idx) = head_qkey_to_idx.get(qkey) {
                counters[idx].1.deleted_lines += 1;
            }
            // Else: function existed in base but not in head (renamed
            // or removed). Counted at file level; not attributed to
            // any per-function row.
        }
    }

    let functions: Vec<FunctionChurn> = counters
        .into_iter()
        .map(|(_, mut c)| {
            c.modified_lines = c.added_lines.min(c.deleted_lines);
            c
        })
        .filter(|c| c.added_lines > 0 || c.deleted_lines > 0)
        .collect();

    FileChurn {
        path: path.to_string(),
        file_added,
        file_deleted,
        functions,
    }
}

/// Diff `base` vs `head` as line streams; return `(added_lines_in_head, deleted_lines_in_base)`
/// — vectors of 1-indexed line numbers.
fn diff_line_numbers(base: &str, head: &str) -> (Vec<u32>, Vec<u32>) {
    let diff = TextDiff::from_lines(base, head);
    let mut added: Vec<u32> = Vec::new();
    let mut deleted: Vec<u32> = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => {
                if let Some(idx) = change.new_index() {
                    added.push(idx as u32 + 1);
                }
            }
            ChangeTag::Delete => {
                if let Some(idx) = change.old_index() {
                    deleted.push(idx as u32 + 1);
                }
            }
            ChangeTag::Equal => {}
        }
    }
    (added, deleted)
}

/// One contiguous changed range of a file. Lines are 1-indexed and
/// inclusive. A pure deletion has no head range and `head_after` is the
/// head line it sits after (0 = before the first line); a pure insertion
/// has no base range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub head: Option<(u32, u32)>,
    pub base: Option<(u32, u32)>,
    pub head_after: Option<u32>,
}

/// The changed ranges of `head` against `base`, in file order. Adjacent
/// deleted and inserted lines are one hunk.
pub fn diff_hunks(base: &str, head: &str) -> Vec<Hunk> {
    let diff = TextDiff::from_lines(base, head);
    let mut hunks = Vec::new();
    // The open hunk: base range and head range as half-open 0-indexed.
    let mut open: Option<(std::ops::Range<usize>, std::ops::Range<usize>)> = None;
    let mut close = |open: &mut Option<(std::ops::Range<usize>, std::ops::Range<usize>)>| {
        if let Some((b, h)) = open.take() {
            hunks.push(Hunk {
                head: (!h.is_empty()).then(|| (h.start as u32 + 1, h.end as u32)),
                base: (!b.is_empty()).then(|| (b.start as u32 + 1, b.end as u32)),
                head_after: h.is_empty().then_some(h.start as u32),
            });
        }
    };
    for op in diff.ops() {
        let (tag, b, h) = op.as_tag_tuple();
        if tag == similar::DiffTag::Equal {
            close(&mut open);
            continue;
        }
        match &mut open {
            Some((ob, oh)) => {
                ob.end = b.end;
                oh.end = h.end;
            }
            None => open = Some((b, h)),
        }
    }
    close(&mut open);
    hunks
}

/// Walk `intervals` (sorted by start_line) and return the index of
/// the first one whose `[start, end]` range contains `line`.
fn locate_interval(intervals: &[(u32, u32, usize)], line: u32) -> Option<usize> {
    for &(start, end, idx) in intervals {
        if start <= line && line <= end {
            return Some(idx);
        }
        if start > line {
            break;
        }
    }
    None
}

fn locate_qkey(intervals: &[(u32, u32, String)], line: u32) -> Option<&str> {
    for (start, end, key) in intervals {
        if *start <= line && line <= *end {
            return Some(key.as_str());
        }
        if *start > line {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_code_metrics::Visibility;

    fn fm(name: &str, start: u32, end: u32) -> FunctionMetrics {
        FunctionMetrics {
            path: "src/lib.rs".into(),
            name: name.into(),
            id: name.into(),
            kind: oxplow_code_metrics::SymbolKind::Function,
            complexity: 1,
            length: end - start + 1,
            parameter_count: 0,
            start_line: start,
            end_line: end,
            container_path: Vec::new(),
            visibility: Visibility::Unknown,
            has_doc: false,
        }
    }

    #[test]
    fn hunks_are_the_contiguous_changed_ranges() {
        // Modify line 2, delete line 4 (c), insert after the last line.
        let base = "a\nb\nc\nd\ne\n";
        let head = "a\nB\nc\ne\nf\n";
        let hunks = diff_hunks(base, head);
        assert_eq!(
            hunks,
            vec![
                Hunk {
                    head: Some((2, 2)),
                    base: Some((2, 2)),
                    head_after: None
                },
                Hunk {
                    head: None,
                    base: Some((4, 4)),
                    head_after: Some(3)
                },
                Hunk {
                    head: Some((5, 5)),
                    base: None,
                    head_after: None
                },
            ]
        );
        assert!(diff_hunks("a\n", "a\n").is_empty());
        // Added file: one insertion; deleted file: one deletion before line 1.
        assert_eq!(
            diff_hunks("", "x\ny\n"),
            vec![Hunk {
                head: Some((1, 2)),
                base: None,
                head_after: None
            }]
        );
        assert_eq!(
            diff_hunks("x\ny\n", ""),
            vec![Hunk {
                head: None,
                base: Some((1, 2)),
                head_after: Some(0)
            }]
        );
    }

    #[test]
    fn diffs_simple_change() {
        let base = "a\nb\nc\n";
        let head = "a\nB\nc\n";
        let (added, deleted) = diff_line_numbers(base, head);
        assert_eq!(added, vec![2]);
        assert_eq!(deleted, vec![2]);
    }

    #[test]
    fn diffs_pure_addition() {
        let base = "a\nc\n";
        let head = "a\nb\nc\n";
        let (added, deleted) = diff_line_numbers(base, head);
        assert_eq!(added, vec![2]);
        assert!(deleted.is_empty());
    }

    #[test]
    fn diffs_pure_deletion() {
        let base = "a\nb\nc\n";
        let head = "a\nc\n";
        let (added, deleted) = diff_line_numbers(base, head);
        assert!(added.is_empty());
        assert_eq!(deleted, vec![2]);
    }

    #[test]
    fn attributes_added_lines_to_containing_head_function() {
        // Head: foo lives at lines 1..=5, bar lives at 6..=10.
        // Add a line inside foo (becomes line 4) and one inside bar
        // (becomes line 9).
        let base = "fn foo() {\n    let a = 1;\n}\nfn bar() {\n    let b = 2;\n}\n";
        let head = "fn foo() {\n    let a = 1;\n    let c = 3;\n}\nfn bar() {\n    let b = 2;\n    let d = 4;\n}\n";
        let head_metrics = vec![fm("foo", 1, 4), fm("bar", 5, 8)];
        let base_metrics = vec![fm("foo", 1, 3), fm("bar", 4, 6)];
        let churn = compute_file_churn("src/lib.rs", &base_metrics, &head_metrics, base, head);
        let foo = churn.functions.iter().find(|f| f.name == "foo").unwrap();
        let bar = churn.functions.iter().find(|f| f.name == "bar").unwrap();
        assert_eq!(foo.added_lines, 1);
        assert_eq!(bar.added_lines, 1);
        assert_eq!(churn.file_added, 2);
    }

    #[test]
    fn deletions_attribute_via_symbol_id_to_head_side() {
        let base = "fn foo() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n";
        let head = "fn foo() {\n    let a = 1;\n}\n";
        let base_metrics = vec![fm("foo", 1, 5)];
        let head_metrics = vec![fm("foo", 1, 3)];
        let churn = compute_file_churn("src/lib.rs", &base_metrics, &head_metrics, base, head);
        let foo = churn.functions.iter().find(|f| f.name == "foo").unwrap();
        assert_eq!(foo.deleted_lines, 2);
        assert_eq!(churn.file_deleted, 2);
    }

    #[test]
    fn modified_lines_is_min_of_added_and_deleted() {
        let base = "fn foo() {\n    a;\n    b;\n}\n";
        let head = "fn foo() {\n    a;\n    B;\n    C;\n}\n";
        let base_metrics = vec![fm("foo", 1, 4)];
        let head_metrics = vec![fm("foo", 1, 5)];
        let churn = compute_file_churn("src/lib.rs", &base_metrics, &head_metrics, base, head);
        let foo = &churn.functions[0];
        assert!(foo.added_lines >= 1);
        assert!(foo.deleted_lines >= 1);
        assert_eq!(foo.modified_lines, foo.added_lines.min(foo.deleted_lines));
    }

    #[test]
    fn deletion_in_removed_function_falls_to_file_level_only() {
        let base = "fn gone() {\n    a;\n    b;\n}\n";
        let head = "";
        let base_metrics = vec![fm("gone", 1, 4)];
        let head_metrics: Vec<FunctionMetrics> = Vec::new();
        let churn = compute_file_churn("src/x.rs", &base_metrics, &head_metrics, base, head);
        assert!(churn.file_deleted > 0);
        assert!(churn.functions.is_empty());
    }

    #[test]
    fn empty_diff_produces_zero_churn() {
        let base = "fn foo() {\n    a;\n}\n";
        let churn = compute_file_churn("src/x.rs", &[], &[], base, base);
        assert_eq!(churn.file_added, 0);
        assert_eq!(churn.file_deleted, 0);
        assert!(churn.functions.is_empty());
    }
}
