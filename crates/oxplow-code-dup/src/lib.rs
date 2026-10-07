//! Function-anchored AST subtree-hash duplicate detector
//! (Deckard-style).
//!
//! # What it detects
//!
//! - **Whole-function clones**: two functions in different files whose
//!   normalized AST hashes match.
//! - **Within-function clones**: a sub-block of one function whose
//!   normalized AST hash matches a sub-block of another function (or
//!   another function entirely).
//!
//! # What it deliberately does NOT detect
//!
//! - Duplicated code outside any function body — JSX/HTML markup,
//!   top-level CSS-style object literals, `const` table declarations,
//!   serde derive blocks. These are usually structural boilerplate
//!   that share AST shape across unrelated files (e.g. two unrelated
//!   `const styles = { ... }` blocks). Restricting the corpus to
//!   function bodies eliminates this entire class of false positive
//!   automatically.
//!
//! # Algorithm
//!
//! 1. Parse each file with tree-sitter.
//! 2. For each file, find every function-like node (per
//!    `Language::spec().function_kinds`).
//! 3. For each function, compute Deckard-style hashes for the
//!    function as a whole AND for every sub-subtree above
//!    `min_nodes`/`min_lines`. See [`tokenize::hash_subtree`].
//! 4. Group records by hash. Each group of >=2 records produces one
//!    cross-pair finding per (a, b) pair of source functions, taking
//!    the LARGEST matching subtree between that pair (so a whole-
//!    function clone subsumes any internal sub-block matches).
//! 5. Sort findings deterministically.

use std::collections::{BTreeSet, HashMap};

use oxplow_code_metrics::{is_function_node, language_for_path, Language};
use tree_sitter::Parser;

mod tokenize;

use tokenize::hash_subtree;

/// One duplicate pair the runner will emit as two findings (one per side).
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateBlock {
    pub a_path: String,
    pub a_start_line: u32,
    pub a_end_line: u32,
    pub b_path: String,
    pub b_start_line: u32,
    pub b_end_line: u32,
    /// Number of lines the two regions span (max of the two).
    pub line_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DupOptions {
    /// Minimum line span to report. Filters trivial single-block
    /// echoes.
    pub min_lines: u32,
    /// Minimum subtree node count to consider for matching. Subtrees
    /// smaller than this never seed a finding — kills the "two
    /// short methods that just call `self.x()`" class of match.
    pub min_nodes: u32,
}

impl Default for DupOptions {
    fn default() -> Self {
        Self {
            // 5 lines surfaces small extracted helpers. Lower-bound
            // noise is held back by min_nodes (subtree must contain
            // 30 AST nodes) plus the function-anchoring (top-level
            // boilerplate isn't in the corpus at all), so the line
            // floor can be aggressive without generating the noise
            // 5 produced under the prior token-window detector.
            min_lines: 5,
            // 30 nodes ≈ a small but nontrivial block (a 5-7 line
            // body with branching). Below this is mostly
            // `return foo()` / property accesses / single
            // expressions.
            min_nodes: 30,
        }
    }
}

/// Detect duplicate function bodies and duplicate sub-blocks
/// within function bodies, across the given files.
///
/// The corpus is **function bodies only** — top-level declarations,
/// const tables, JSX expression trees outside a function, etc. are
/// ignored on purpose. See module docs.
///
/// Same-file matches are emitted (a function in F1 can clone a
/// different function in F1). Callers that don't want self-matches
/// should use [`detect_duplicates_scoped`].
///
/// Files in unsupported languages are skipped silently.
pub fn detect_duplicates<I, P, S>(files: I, opts: DupOptions) -> Vec<DuplicateBlock>
where
    I: IntoIterator<Item = (P, S)>,
    P: Into<String>,
    S: AsRef<str>,
{
    let _running = Running::start();
    let mut subtrees: Vec<SubtreeRef> = Vec::new();
    for (path, source) in files {
        let path = path.into();
        let Some(lang) = language_for_path(&path) else {
            continue;
        };
        let source = source.as_ref();
        let mut parser = Parser::new();
        if parser.set_language(&lang.tree_sitter_language()).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(source, None) else {
            continue;
        };
        collect_function_subtrees(
            &path,
            lang,
            tree.root_node(),
            source.as_bytes(),
            opts,
            &mut subtrees,
        );
    }
    pair_up(subtrees, opts)
}

#[cfg(test)]
static RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The most detections that ran at once (tests).
#[cfg(test)]
pub(crate) fn peak_running() -> usize {
    PEAK.load(std::sync::atomic::Ordering::SeqCst)
}

/// Held by the detection running: one at a time per process. A detection
/// parses its whole corpus (the tree) and is CPU-bound, so two at once — a
/// change's scan and the whole-tree `oxplow.duplicate_lines` collector's —
/// only compete for the same cores and the person's typing; the second
/// waits for the first.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One detection running, for as long as it's held.
struct Running {
    _held: std::sync::MutexGuard<'static, ()>,
}

impl Running {
    fn start() -> Self {
        let held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering::SeqCst;
            let now = RUNNING.fetch_add(1, SeqCst) + 1;
            PEAK.fetch_max(now, SeqCst);
        }
        Running { _held: held }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        #[cfg(test)]
        RUNNING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Same as [`detect_duplicates`] but applies "scope" semantics:
///
/// - The whole corpus participates in matching (so a changed file
///   can clone-match an unchanged peer file).
/// - A finding is reported only if **at least one** side's path is
///   in `scope_paths`.
/// - Same-path matches (file vs itself) are dropped.
/// - When only one side is in scope it is rotated to the A side
///   so the renderer's "side A is what you're analyzing" convention
///   holds.
pub fn detect_duplicates_scoped<I, P, S>(
    files: I,
    scope_paths: &BTreeSet<String>,
    opts: DupOptions,
) -> Vec<DuplicateBlock>
where
    I: IntoIterator<Item = (P, S)>,
    P: Into<String>,
    S: AsRef<str>,
{
    let raw = detect_duplicates(files, opts);
    let mut out = Vec::with_capacity(raw.len());
    for block in raw {
        if block.a_path == block.b_path {
            continue;
        }
        let a_in = scope_paths.contains(&block.a_path);
        let b_in = scope_paths.contains(&block.b_path);
        if !a_in && !b_in {
            continue;
        }
        if !a_in && b_in {
            out.push(DuplicateBlock {
                a_path: block.b_path,
                a_start_line: block.b_start_line,
                a_end_line: block.b_end_line,
                b_path: block.a_path,
                b_start_line: block.a_start_line,
                b_end_line: block.a_end_line,
                line_count: block.line_count,
            });
        } else {
            out.push(block);
        }
    }
    out
}

/// One subtree extracted from the corpus that's eligible for
/// matching. Each record carries enough context to emit a finding
/// directly (path + line range) and to detect "this match is nested
/// inside a larger one" via byte offsets within the parent function.
#[derive(Debug, Clone)]
struct SubtreeRef {
    path: String,
    /// Byte range of the enclosing function body. Used to detect
    /// whether two SubtreeRefs come from the same source function,
    /// so we can collapse "whole function" + "loop inside that
    /// function" matches into the single largest one per pair.
    fn_byte_start: usize,
    fn_byte_end: usize,
    start_line: u32,
    end_line: u32,
    node_count: u32,
    hash: u64,
}

/// Walk the AST top-down. Whenever we hit a function-like node,
/// hash its body subtree AND every internal subtree large enough
/// to match. Recurse into nested functions (closures inside
/// methods, etc.) so they get their own records.
fn collect_function_subtrees(
    path: &str,
    lang: Language,
    root: tree_sitter::Node<'_>,
    src: &[u8],
    opts: DupOptions,
    out: &mut Vec<SubtreeRef>,
) {
    walk_for_functions(path, lang, root, src, opts, out);
}

fn walk_for_functions(
    path: &str,
    lang: Language,
    node: tree_sitter::Node<'_>,
    src: &[u8],
    opts: DupOptions,
    out: &mut Vec<SubtreeRef>,
) {
    let spec = lang.spec();
    if is_function_node(node, src, spec) {
        record_function(path, lang, node, src, opts, out);
        // Continue walking children — there may be nested functions
        // (closures, inner functions). They'll get their own records.
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_for_functions(path, lang, cursor.node(), src, opts, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Hash this function's body and every internal subtree that's big
/// enough to seed a meaningful match.
fn record_function(
    path: &str,
    lang: Language,
    fn_node: tree_sitter::Node<'_>,
    src: &[u8],
    opts: DupOptions,
    out: &mut Vec<SubtreeRef>,
) {
    let fn_byte_start = fn_node.start_byte();
    let fn_byte_end = fn_node.end_byte();
    collect_subtrees_recursive(
        path,
        lang,
        fn_node,
        src,
        fn_byte_start,
        fn_byte_end,
        opts,
        out,
        /* is_root */ true,
    );
}

#[allow(clippy::too_many_arguments)]
fn collect_subtrees_recursive(
    path: &str,
    lang: Language,
    node: tree_sitter::Node<'_>,
    src: &[u8],
    fn_byte_start: usize,
    fn_byte_end: usize,
    opts: DupOptions,
    out: &mut Vec<SubtreeRef>,
    is_root: bool,
) {
    let spec = lang.spec();
    // Don't re-stamp subtrees of nested functions onto the outer
    // function's FnId — `walk_for_functions` will visit each
    // nested function as its own root and produce its own
    // SubtreeRefs. Without this gate the same physical subtree
    // ends up in `subtrees` once per ancestor function, and
    // pair_up emits a DuplicateBlock for every (outer_fn, inner_fn)
    // ancestry combination — visible as identical line-range
    // rows repeating in the duplication panel.
    if !is_root && is_function_node(node, src, spec) {
        return;
    }
    let h = hash_subtree(node, src, lang);
    let line_span = h.end_line.saturating_sub(h.start_line) + 1;
    if h.node_count >= opts.min_nodes && line_span >= opts.min_lines {
        out.push(SubtreeRef {
            path: path.to_string(),
            fn_byte_start,
            fn_byte_end,
            start_line: h.start_line,
            end_line: h.end_line,
            node_count: h.node_count,
            hash: h.hash,
        });
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if !tokenize::is_comment_kind(child.kind()) && !tokenize::is_skip_node(lang, child, src)
            {
                collect_subtrees_recursive(
                    path,
                    lang,
                    child,
                    src,
                    fn_byte_start,
                    fn_byte_end,
                    opts,
                    out,
                    /* is_root */ false,
                );
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Group subtree records by hash; for each group with >=2 entries,
/// emit the largest matching subtree per (function-A, function-B)
/// pair. The "function" identity is the parent function's byte
/// range within its file — so two subtrees in the SAME function are
/// not a pair (we wouldn't surface "lines 10-15 ↔ lines 20-25 of
/// the same function" as a duplicate; if those are actually
/// duplicates inside one function, that's a refactor concern but
/// not what the change-analysis flow surfaces).
fn pair_up(subtrees: Vec<SubtreeRef>, _opts: DupOptions) -> Vec<DuplicateBlock> {
    let mut by_hash: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in subtrees.iter().enumerate() {
        by_hash.entry(s.hash).or_default().push(i);
    }
    // Best (largest by node_count) match per ordered (a_fn, b_fn) pair.
    type FnId<'a> = (&'a str, usize, usize); // (path, fn_byte_start, fn_byte_end)
    let mut best: HashMap<(FnId, FnId), (u32, usize, usize)> = HashMap::new();
    for idxs in by_hash.values() {
        if idxs.len() < 2 {
            continue;
        }
        for i in 0..idxs.len() {
            for j in (i + 1)..idxs.len() {
                let a = &subtrees[idxs[i]];
                let b = &subtrees[idxs[j]];
                // Same source function — not a cross-function clone.
                if a.path == b.path
                    && a.fn_byte_start == b.fn_byte_start
                    && a.fn_byte_end == b.fn_byte_end
                {
                    continue;
                }
                let (lo, hi) = order(a, b, idxs[i], idxs[j]);
                let key = (
                    (lo.0.path.as_str(), lo.0.fn_byte_start, lo.0.fn_byte_end),
                    (hi.0.path.as_str(), hi.0.fn_byte_start, hi.0.fn_byte_end),
                );
                let candidate_size = lo.0.node_count.min(hi.0.node_count);
                let entry = best.entry(key).or_insert((0, lo.1, hi.1));
                if candidate_size > entry.0 {
                    *entry = (candidate_size, lo.1, hi.1);
                }
            }
        }
    }

    let mut out: Vec<DuplicateBlock> = best
        .into_values()
        .map(|(_size, a_idx, b_idx)| {
            let a = &subtrees[a_idx];
            let b = &subtrees[b_idx];
            let span_a = a.end_line.saturating_sub(a.start_line) + 1;
            let span_b = b.end_line.saturating_sub(b.start_line) + 1;
            DuplicateBlock {
                a_path: a.path.clone(),
                a_start_line: a.start_line,
                a_end_line: a.end_line,
                b_path: b.path.clone(),
                b_start_line: b.start_line,
                b_end_line: b.end_line,
                line_count: span_a.max(span_b),
            }
        })
        .collect();
    out.sort_by(|x, y| {
        x.a_path
            .cmp(&y.a_path)
            .then_with(|| x.a_start_line.cmp(&y.a_start_line))
            .then_with(|| x.b_path.cmp(&y.b_path))
            .then_with(|| x.b_start_line.cmp(&y.b_start_line))
    });
    out
}

/// Produce a stable ordering for a pair of records so we hash to
/// one canonical (a, b) key regardless of which side we encountered
/// first. Returns ((lo_record, lo_idx), (hi_record, hi_idx)).
fn order<'a>(
    a: &'a SubtreeRef,
    b: &'a SubtreeRef,
    a_idx: usize,
    b_idx: usize,
) -> ((&'a SubtreeRef, usize), (&'a SubtreeRef, usize)) {
    let a_key = (&a.path, a.fn_byte_start, a.fn_byte_end);
    let b_key = (&b.path, b.fn_byte_start, b.fn_byte_end);
    if a_key <= b_key {
        ((a, a_idx), (b, b_idx))
    } else {
        ((b, b_idx), (a, a_idx))
    }
}

#[cfg(test)]
mod tests;
