//! Tier-1 smart conflict auto-resolution: a language-agnostic
//! **token-level** 3-way merge.
//!
//! Git's merge driver works at LINE granularity, so two edits that touch
//! *different words on the same line* land in the same line-block and are
//! reported as a conflict even though they don't actually overlap. This
//! is exactly what IntelliJ's "magic wand / resolve simple conflicts"
//! fixes by comparing at word granularity. We reproduce that here: split
//! base / ours / theirs into tokens (word runs, whitespace runs, newlines,
//! and individual punctuation chars) and run a classic diff3 over the
//! token streams instead of lines.
//!
//! ## Safety model (never auto-resolve a true overlap)
//!
//! [`merge3`] returns `Err(Conflicted)` whenever ours and theirs change
//! the *same* base tokens in *different* ways (including delete-vs-modify
//! and add/add of different text at the same point). The driver
//! ([`auto_resolve_conflicts`], in a later change) only writes + stages a
//! file when `merge3` returns `Ok`. Tokenization is lossless
//! (`join(tokenize(s)) == s`), so a clean merge reproduces every
//! non-conflicting byte exactly. Git has already failed its line merge by
//! the time we run, so the worst case is identical to today's behaviour —
//! we only ever *reduce* the conflict count, never introduce new content.

use std::path::Path;

use similar::{capture_diff_slices, Algorithm, DiffOp};

/// Files larger than this are skipped by the conflict driver — they're
/// likely generated/data and word-level merging buys little while risking
/// surprise. Real source conflicts are far smaller.
const MAX_RESOLVE_BYTES: usize = 1 << 20; // 1 MiB

/// Returned by [`merge3`] when ours and theirs make incompatible changes
/// to an overlapping region of the base. The file is left untouched so
/// the user resolves git's original markers by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflicted;

/// True iff `c` is part of a "word" run (kept together as one token).
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Split `s` into tokens for finer-grained-than-line diffing.
///
/// Token classes: a run of word chars, a run of non-newline whitespace, a
/// single `\n` (so line structure survives), or a single other char. The
/// split is lossless: `tokenize(s).concat() == s` for all `s`.
pub fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c == '\n' {
            out.push(&s[start..start + 1]);
            chars.next();
        } else if is_word(c) {
            let mut end = start + c.len_utf8();
            chars.next();
            while let Some(&(i, c2)) = chars.peek() {
                if is_word(c2) {
                    end = i + c2.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(&s[start..end]);
        } else if c.is_whitespace() {
            // Non-newline whitespace run (newlines are handled above and
            // break the run, so line boundaries stay as their own tokens).
            let mut end = start + c.len_utf8();
            chars.next();
            while let Some(&(i, c2)) = chars.peek() {
                if c2 != '\n' && c2.is_whitespace() {
                    end = i + c2.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(&s[start..end]);
        } else {
            let end = start + c.len_utf8();
            out.push(&s[start..end]);
            chars.next();
        }
    }
    out
}

/// One contiguous edit against the base, expressed as a base range
/// `[start, end)` replaced by `new`. An insertion has `start == end`.
struct Change<T> {
    start: usize,
    end: usize,
    new: Vec<T>,
    /// `false` = from "ours", `true` = from "theirs".
    theirs: bool,
}

/// Convert a base→side diff into the non-equal [`Change`]s it implies.
fn changes_from_ops<T: Clone>(ops: &[DiffOp], side: &[T], theirs: bool) -> Vec<Change<T>> {
    let mut out = Vec::new();
    for op in ops {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index, old_len, ..
            } => out.push(Change {
                start: old_index,
                end: old_index + old_len,
                new: Vec::new(),
                theirs,
            }),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => out.push(Change {
                start: old_index,
                end: old_index,
                new: side[new_index..new_index + new_len].to_vec(),
                theirs,
            }),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => out.push(Change {
                start: old_index,
                end: old_index + old_len,
                new: side[new_index..new_index + new_len].to_vec(),
                theirs,
            }),
        }
    }
    out
}

/// Reconstruct one side's tokens over the base range `[lo, hi)` by
/// applying only that side's changes from `cluster`.
fn reconstruct<T: Clone>(
    base: &[T],
    lo: usize,
    hi: usize,
    cluster: &[&Change<T>],
    theirs: bool,
) -> Vec<T> {
    let mut out = Vec::new();
    let mut pos = lo;
    for ch in cluster.iter().filter(|c| c.theirs == theirs) {
        // Changes are ordered by start and confined to [lo, hi).
        out.extend_from_slice(&base[pos..ch.start]);
        out.extend(ch.new.iter().cloned());
        pos = ch.end;
    }
    out.extend_from_slice(&base[pos..hi]);
    out
}

/// Classic diff3 over arbitrary token slices.
///
/// Diffs ours and theirs against the common `base`, then walks the base
/// merging change regions. A region where both sides edit overlapping
/// base tokens is taken iff the two sides agree (or one side left the
/// region unchanged); otherwise it is a [`Conflicted`] error.
pub fn merge3<T: Clone + Eq + std::hash::Hash + Ord>(
    base: &[T],
    ours: &[T],
    theirs: &[T],
) -> Result<Vec<T>, Conflicted> {
    let ours_ops = capture_diff_slices(Algorithm::Myers, base, ours);
    let theirs_ops = capture_diff_slices(Algorithm::Myers, base, theirs);

    let mut changes: Vec<Change<T>> = changes_from_ops(&ours_ops, ours, false);
    changes.extend(changes_from_ops(&theirs_ops, theirs, true));
    // Order by base position; zero-width inserts sort before the deletion
    // that may start at the same index so they apply first.
    changes.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let mut out: Vec<T> = Vec::new();
    let mut base_pos = 0usize; // next un-emitted base index
    let mut i = 0usize;
    while i < changes.len() {
        // Start a cluster at the next change.
        let lo = changes[i].start;
        let mut hi = changes[i].end;
        let cluster_is_pure_insert = changes[i].start == changes[i].end;
        let mut j = i + 1;
        while j < changes.len() {
            let next = &changes[j];
            let overlaps = next.start < hi;
            // Two zero-width inserts at the very same gap collide (add/add):
            // pull the second into this cluster so we can compare them.
            let same_gap_insert =
                cluster_is_pure_insert && next.start == hi && next.start == next.end;
            if overlaps || same_gap_insert {
                hi = hi.max(next.end);
                j += 1;
            } else {
                break;
            }
        }

        // Emit untouched base before the cluster.
        out.extend_from_slice(&base[base_pos..lo]);

        let cluster: Vec<&Change<T>> = changes[i..j].iter().collect();
        let ours_repl = reconstruct(base, lo, hi, &cluster, false);
        let theirs_repl = reconstruct(base, lo, hi, &cluster, true);
        let base_slice = &base[lo..hi];

        let resolved = if ours_repl == theirs_repl {
            ours_repl
        } else if ours_repl.as_slice() == base_slice {
            theirs_repl
        } else if theirs_repl.as_slice() == base_slice {
            ours_repl
        } else {
            return Err(Conflicted);
        };
        out.extend(resolved);

        base_pos = hi;
        i = j;
    }
    out.extend_from_slice(&base[base_pos..]);
    Ok(out)
}

/// Convenience wrapper: tokenize the three revisions, [`merge3`] them, and
/// re-join. This is what the conflict driver uses on file contents.
pub fn merge3_str(base: &str, ours: &str, theirs: &str) -> Result<String, Conflicted> {
    let merged = merge3(&tokenize(base), &tokenize(ours), &tokenize(theirs))?;
    Ok(merged.concat())
}

/// Outcome of an [`auto_resolve_conflicts`] pass.
#[derive(Debug, Default, Clone)]
pub struct AutoResolveReport {
    /// Repo-relative paths that were cleanly auto-resolved and staged —
    /// across *both* tiers (token diff3 and the AST structural pass).
    pub resolved: Vec<String>,
    /// How many of `resolved` were cleared by the Tier-2 AST structural
    /// pass (the rest came from Tier-1's token diff3). Lets the UI/tests
    /// distinguish the two without a parallel code path.
    pub ast_resolved: u32,
    /// Unmerged paths still left conflicted after the pass (genuine
    /// overlaps, add/add, delete/modify, binary, oversized, …).
    pub remaining: u32,
}

/// Read a blob as UTF-8 text, refusing binary / oversized content.
fn blob_text(repo: &git2::Repository, id: git2::Oid) -> Option<String> {
    let blob = repo.find_blob(id).ok()?;
    let content = blob.content();
    if content.len() > MAX_RESOLVE_BYTES {
        return None;
    }
    std::str::from_utf8(content).ok().map(|s| s.to_owned())
}

/// After git has left conflicts, try to auto-resolve the ones that are
/// only conflicts at LINE granularity by running [`merge3_str`] on each
/// file's base/ours/theirs stages. A file is written + `git add`ed only
/// when its token-level merge is unambiguous (`Ok`); genuine overlaps,
/// add/add, delete/modify, binary, and oversized files are left exactly
/// as git produced them (markers intact) for the user to resolve.
///
/// This is the IntelliJ-magic-wand pass: it can only *reduce* the
/// conflict count, never introduce new content.
pub fn auto_resolve_conflicts(repo_path: &Path) -> AutoResolveReport {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return AutoResolveReport::default();
    };
    let Ok(index) = repo.index() else {
        return AutoResolveReport::default();
    };

    // Collect candidate (path, base, ours, theirs) tuples first so we
    // don't hold the index borrow while writing files / shelling `git add`.
    let mut candidates: Vec<(String, String, String, String)> = Vec::new();
    if let Ok(conflicts) = index.conflicts() {
        for conflict in conflicts.flatten() {
            // Only modify/modify (all three stages present) is in scope.
            let (Some(ancestor), Some(our), Some(their)) =
                (conflict.ancestor, conflict.our, conflict.their)
            else {
                continue;
            };
            let Ok(path) = std::str::from_utf8(&our.path) else {
                continue;
            };
            let (Some(base), Some(ours), Some(theirs)) = (
                blob_text(&repo, ancestor.id),
                blob_text(&repo, our.id),
                blob_text(&repo, their.id),
            ) else {
                continue;
            };
            candidates.push((path.to_owned(), base, ours, theirs));
        }
    }
    drop(index);

    let mut resolved = Vec::new();
    let mut ast_resolved = 0u32;
    for (path, base, ours, theirs) in candidates {
        // Tier-1: language-agnostic token diff3. Tier-2 (AST structural):
        // only when Tier-1 leaves a conflict *and* the path maps to a
        // supported grammar whose 3-way merge reconstructs + re-parses
        // cleanly. Both tiers can only ever *reduce* the conflict count.
        let merged = match merge3_str(&base, &ours, &theirs) {
            Ok(m) => Some((m, false)),
            Err(_) => ast_merge_resolve(&path, &base, &ours, &theirs).map(|m| (m, true)),
        };
        let Some((merged, via_ast)) = merged else {
            continue; // neither tier could resolve — leave git's markers.
        };
        let abs = repo_path.join(&path);
        if std::fs::write(&abs, merged).is_err() {
            continue;
        }
        // Stage at slot 0, which clears the unmerged stages for this path.
        let staged = crate::sync::add_path(repo_path, &path)
            .map(|r| r.success)
            .unwrap_or(false);
        if staged {
            if via_ast {
                ast_resolved += 1;
            }
            resolved.push(path);
        }
    }

    // Recount remaining conflicts from a freshly-read index.
    let remaining = repo
        .index()
        .ok()
        .and_then(|mut idx| {
            idx.read(true).ok()?;
            Some(idx.conflicts().map(|c| c.count() as u32).unwrap_or(0))
        })
        .unwrap_or(0);

    AutoResolveReport {
        resolved,
        ast_resolved,
        remaining,
    }
}

/// Tier-2 fallback: when Tier-1 can't merge a file, try the AST
/// structural pass iff the path maps to a supported grammar. Returns the
/// reconstructed content only when [`merge_top_level`] resolves cleanly
/// (which already requires the re-parse guard to pass); any conflict,
/// bail, parse failure, or unsupported extension yields `None`, leaving
/// git's markers. Preserves the reduce-only invariant.
///
/// [`merge_top_level`]: crate::ast_merge::merge_top_level
fn ast_merge_resolve(path: &str, base: &str, ours: &str, theirs: &str) -> Option<String> {
    let lang = crate::ast_merge::language_for_path(path)?;
    match crate::ast_merge::merge_top_level(base, ours, theirs, lang) {
        crate::ast_merge::AstMerge::Resolved(text) => Some(text),
        crate::ast_merge::AstMerge::Conflict(_) | crate::ast_merge::AstMerge::Bail(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        tokenize(s).into_iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn tokenize_round_trips() {
        for s in [
            "",
            "hello",
            "foo bar baz\n",
            "let x = foo_bar + 1;\n",
            "a\r\nb\r\n",
            "café — résumé\n\tindented",
            "用户 = get()\n",
            "multiple   spaces\tand\ttabs",
        ] {
            assert_eq!(tokenize(s).concat(), s, "round-trip failed for {s:?}");
        }
    }

    #[test]
    fn tokenize_keeps_newlines_as_own_tokens() {
        assert_eq!(tokenize("a\nb"), vec!["a", "\n", "b"]);
    }

    #[test]
    fn different_words_same_line_merges() {
        // The flagship case git leaves conflicted: both sides edit the same
        // line but different words.
        let base = "foo bar baz\n";
        let ours = "FOO bar baz\n";
        let theirs = "foo bar BAZ\n";
        assert_eq!(merge3_str(base, ours, theirs).unwrap(), "FOO bar BAZ\n");
    }

    #[test]
    fn both_add_different_imports_merges() {
        let base = "use std::fmt;\n";
        let ours = "use std::collections::HashMap;\nuse std::fmt;\n";
        let theirs = "use std::fmt;\nuse std::io;\n";
        let merged = merge3_str(base, ours, theirs).unwrap();
        assert!(merged.contains("HashMap"), "merged = {merged:?}");
        assert!(merged.contains("std::io"), "merged = {merged:?}");
        assert!(merged.contains("std::fmt"), "merged = {merged:?}");
    }

    #[test]
    fn same_token_different_change_conflicts() {
        let base = "let timeout = 10;\n";
        let ours = "let timeout = 20;\n";
        let theirs = "let timeout = 30;\n";
        assert_eq!(merge3_str(base, ours, theirs), Err(Conflicted));
    }

    #[test]
    fn one_side_only_takes_that_side() {
        let base = "alpha beta gamma";
        let ours = "alpha BETA gamma";
        let theirs = "alpha beta gamma";
        assert_eq!(merge3_str(base, ours, theirs).unwrap(), "alpha BETA gamma");
        // Symmetric: only theirs changed.
        assert_eq!(merge3_str(base, theirs, ours).unwrap(), "alpha BETA gamma");
    }

    #[test]
    fn identical_change_both_sides_takes_once() {
        let base = "value = 1";
        let ours = "value = 2";
        let theirs = "value = 2";
        assert_eq!(merge3_str(base, ours, theirs).unwrap(), "value = 2");
    }

    #[test]
    fn delete_vs_modify_conflicts() {
        // ours deletes the word, theirs changes it: a true overlap.
        let base = vec!["keep", "DROP", "keep2"];
        let ours = vec!["keep", "keep2"];
        let theirs = vec!["keep", "CHANGED", "keep2"];
        assert_eq!(merge3(&base, &ours, &theirs), Err(Conflicted));
    }

    #[test]
    fn disjoint_inserts_both_applied() {
        // Insertions at different base positions both land.
        let base = vec!["a", "b", "c"];
        let ours = vec!["a", "X", "b", "c"];
        let theirs = vec!["a", "b", "c", "Y"];
        assert_eq!(
            merge3(&base, &ours, &theirs).unwrap(),
            vec!["a", "X", "b", "c", "Y"]
        );
    }

    #[test]
    fn dual_insert_same_point_different_text_conflicts() {
        // Both sides insert *different* text at the same gap: ambiguous
        // order, so we refuse rather than guess.
        let base = vec!["a", "b"];
        let ours = vec!["a", "X", "b"];
        let theirs = vec!["a", "Y", "b"];
        assert_eq!(merge3(&base, &ours, &theirs), Err(Conflicted));
    }

    #[test]
    fn dual_insert_same_point_same_text_merges() {
        let base = vec!["a", "b"];
        let ours = vec!["a", "X", "b"];
        let theirs = vec!["a", "X", "b"];
        assert_eq!(merge3(&base, &ours, &theirs).unwrap(), vec!["a", "X", "b"]);
    }

    #[test]
    fn no_changes_returns_base() {
        let base = toks("unchanged\n");
        assert_eq!(merge3(&base, &base, &base).unwrap(), base);
    }

    #[test]
    fn adjacent_replace_then_insert_both_applied() {
        // ours replaces [1,3), theirs inserts after index 3 — adjacent, not
        // overlapping; both should apply without conflict.
        let base = vec!["a", "b", "c", "d"];
        let ours = vec!["a", "B", "C", "d"];
        let theirs = vec!["a", "b", "c", "Z", "d"];
        assert_eq!(
            merge3(&base, &ours, &theirs).unwrap(),
            vec!["a", "B", "C", "Z", "d"]
        );
    }

    // --- End-to-end against a real git index -----------------------------

    fn run_git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit(dir: &Path, contents: &str, msg: &str) {
        std::fs::write(dir.join("cfg.txt"), contents).unwrap();
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", msg]);
    }

    /// `git revert` leaving a line-level conflict (the reverted commit and
    /// HEAD touched different words of the same line) is cleared by the
    /// op-agnostic auto-resolve pass.
    #[test]
    fn auto_resolve_clears_revert_conflict_with_disjoint_edit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        run_git(p, &["init", "-q", "--initial-branch=main"]);
        run_git(p, &["config", "user.email", "t@example.com"]);
        run_git(p, &["config", "user.name", "t"]);
        commit(p, "alpha beta gamma\n", "base");
        commit(p, "alpha beta GAMMA\n", "c2: word3"); // the commit we revert
        let c2 = crate::status::head_commit_sha(p).unwrap();
        commit(p, "ALPHA beta GAMMA\n", "c3: word1");

        // Reverting c2 wants GAMMA→gamma, but HEAD changed word1 → git
        // reports a conflict.
        let r = crate::sync::revert(p, &c2).unwrap();
        assert!(!r.success, "expected git revert to conflict");
        assert_eq!(count_unmerged(p), 1);

        let report = auto_resolve_conflicts(p);
        assert_eq!(report.resolved.len(), 1);
        assert_eq!(report.remaining, 0);
        assert_eq!(
            std::fs::read_to_string(p.join("cfg.txt")).unwrap(),
            "ALPHA beta gamma\n"
        );
    }

    fn count_unmerged(p: &Path) -> u32 {
        let repo = git2::Repository::open(p).unwrap();
        let idx = repo.index().unwrap();
        idx.conflicts().map(|c| c.count() as u32).unwrap_or(0)
    }

    fn init_repo(p: &Path) {
        run_git(p, &["init", "-q", "--initial-branch=main"]);
        run_git(p, &["config", "user.email", "t@example.com"]);
        run_git(p, &["config", "user.name", "t"]);
    }

    /// Commit `contents` to `file`, staging everything.
    fn commit_file(p: &Path, file: &str, contents: &str, msg: &str) {
        std::fs::write(p.join(file), contents).unwrap();
        run_git(p, &["add", "-A"]);
        run_git(p, &["commit", "-q", "-m", msg]);
    }

    /// Both sides add a *different* import at the same gap (adjacent lines)
    /// — git conflicts and Tier-1's token diff3 sees an ambiguous add/add,
    /// but the AST structural tier recognizes two independent imports and
    /// unions them. The report attributes the win to the AST tier.
    #[test]
    fn auto_resolve_clears_ast_only_import_conflict() {
        let base = "use std::fmt;\nfn main() {}\n";
        let ours = "use std::fmt;\nuse std::io;\nfn main() {}\n";
        let theirs = "use std::fmt;\nuse std::cmp;\nfn main() {}\n";

        // Precondition: Tier-1 genuinely can't merge this (adjacent add/add).
        assert_eq!(
            merge3_str(base, ours, theirs),
            Err(Conflicted),
            "Tier-1 must leave this conflicted for the test to exercise Tier-2"
        );

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        commit_file(p, "lib.rs", base, "base");
        run_git(p, &["checkout", "-q", "-b", "feature"]);
        commit_file(p, "lib.rs", ours, "ours: add io");
        run_git(p, &["checkout", "-q", "main"]);
        commit_file(p, "lib.rs", theirs, "theirs: add cmp");

        let out = std::process::Command::new("git")
            .args(["merge", "--no-edit", "feature"])
            .current_dir(p)
            .output()
            .expect("spawn git merge");
        assert!(!out.status.success(), "expected git merge to conflict");
        assert_eq!(count_unmerged(p), 1);

        let report = auto_resolve_conflicts(p);
        assert_eq!(report.resolved.len(), 1, "{report:?}");
        assert_eq!(report.ast_resolved, 1, "AST tier should own the win");
        assert_eq!(report.remaining, 0);

        let merged = std::fs::read_to_string(p.join("lib.rs")).unwrap();
        assert!(merged.contains("use std::io;"), "{merged}");
        assert!(merged.contains("use std::cmp;"), "{merged}");
        assert!(merged.contains("use std::fmt;"), "{merged}");
        assert!(merged.contains("fn main()"), "{merged}");
        // The reconstruction must itself parse.
        assert!(
            crate::ast_merge::parse_top_level_items(&merged, crate::ast_merge::Language::Rust)
                .is_some(),
            "AST reconstruction must re-parse: {merged}"
        );
    }

    /// An unsupported extension gets no AST tier: an add/add conflict
    /// Tier-1 also can't resolve is left exactly as git produced it.
    #[test]
    fn auto_resolve_leaves_unsupported_extension_conflict() {
        let base = "line one\nline two\n";
        let ours = "line one\nadded by ours\nline two\n";
        let theirs = "line one\nadded by theirs\nline two\n";
        assert_eq!(
            merge3_str(base, ours, theirs),
            Err(Conflicted),
            "precondition: Tier-1 can't resolve adjacent add/add"
        );

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init_repo(p);
        commit_file(p, "notes.txt", base, "base");
        run_git(p, &["checkout", "-q", "-b", "feature"]);
        commit_file(p, "notes.txt", ours, "ours");
        run_git(p, &["checkout", "-q", "main"]);
        commit_file(p, "notes.txt", theirs, "theirs");

        let out = std::process::Command::new("git")
            .args(["merge", "--no-edit", "feature"])
            .current_dir(p)
            .output()
            .expect("spawn git merge");
        assert!(!out.status.success(), "expected git merge to conflict");
        assert_eq!(count_unmerged(p), 1);

        let report = auto_resolve_conflicts(p);
        assert!(report.resolved.is_empty(), "{report:?}");
        assert_eq!(report.ast_resolved, 0);
        assert_eq!(report.remaining, 1, "unsupported ext left for the user");
    }
}
