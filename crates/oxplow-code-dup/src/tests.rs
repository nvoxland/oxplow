use super::*;

/// Detection-mechanic tests use short fixtures (~5-line bodies); the
/// production default of 10 would filter them out. Lower the bar to 5
/// so we test what we mean to test (k-gram matching, rename
/// resilience, skip tolerance, scope semantics) without the
/// boilerplate-suppression knob obscuring the result.
fn detect_opts() -> DupOptions {
    DupOptions {
        min_lines: 5,
        ..DupOptions::default()
    }
}

#[test]
fn detects_obvious_clone_across_two_files() {
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    let files = vec![
        ("src/a.rs".to_string(), body.to_string()),
        ("src/b.rs".to_string(), body.to_string()),
    ];
    let blocks = detect_duplicates(files, detect_opts());
    assert!(!blocks.is_empty(), "expected at least one duplicate");
    let b = &blocks[0];
    assert_eq!(b.a_path, "src/a.rs");
    assert_eq!(b.b_path, "src/b.rs");
    assert!(b.line_count >= 5);
}

#[test]
fn rename_resistant_clones() {
    let original = r#"
fn process(items: Vec<i32>) -> Vec<i32> {
    let mut result = Vec::new();
    for thing in items {
        if thing > 0 {
            result.push(thing * 2);
        } else if thing < 0 {
            result.push(thing * -1);
        } else {
            result.push(0);
        }
    }
    result
}
"#;
    let renamed = r#"
fn handle(values: Vec<i32>) -> Vec<i32> {
    let mut output = Vec::new();
    for v in values {
        if v > 0 {
            output.push(v * 2);
        } else if v < 0 {
            output.push(v * -1);
        } else {
            output.push(0);
        }
    }
    output
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), original.to_string()),
            ("src/b.rs".to_string(), renamed.to_string()),
        ],
        detect_opts(),
    );
    assert!(!blocks.is_empty(), "renamed clones should still match");
}

#[test]
fn ignores_unique_files() {
    let a = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let b = "fn unrelated() { println!(\"hello world\"); }";
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}

#[test]
fn ignores_short_clones_below_min_lines() {
    let body = "fn tiny() { println!(\"x\"); }";
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), body.to_string()),
            ("src/b.rs".to_string(), body.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty(), "single-line clone should be filtered");
}

#[test]
fn cross_language_files_dont_match() {
    // Even if textually similar, different languages produce
    // different token kinds and shouldn't cross-match.
    let rs = r#"fn add(a: i32, b: i32) -> i32 { if a > b { a } else { b } }"#;
    let py = r#"def add(a, b):
    if a > b:
        return a
    else:
        return b
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), rs.to_string()),
            ("src/b.py".to_string(), py.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}

#[test]
fn language_salt_prevents_cross_language_collisions() {
    // Two grammars producing structurally similar token sequences
    // with overlapping node-kind names should never share fingerprints.
    // We construct C and Rust functions whose normalized token streams
    // overlap heavily ("if", "(", "ID", ")", "{", ... "}").
    let c_src = r#"
int a(int x) {
    if (x > 0) { return 1; }
    if (x > 1) { return 2; }
    if (x > 2) { return 3; }
    if (x > 3) { return 4; }
    return 0;
}
"#;
    let rust_src = r#"
fn a(x: i32) -> i32 {
    if x > 0 { return 1; }
    if x > 1 { return 2; }
    if x > 2 { return 3; }
    if x > 3 { return 4; }
    0
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/x.c".to_string(), c_src.to_string()),
            ("src/x.rs".to_string(), rust_src.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(
        blocks.is_empty(),
        "cross-language fingerprints should be salted apart, got {:?}",
        blocks
    );
}

#[test]
fn extend_tolerates_minor_winnowing_divergence() {
    // Two clones with a one-token difference in the middle should
    // still be detected as one block (skip-tolerance), not two
    // smaller fragments or none at all.
    let a = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out.push(99);
    out
}
"#;
    let b = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out.push(100);
    out
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        detect_opts(),
    );
    assert!(!blocks.is_empty(), "near-clone should still match");
}

#[test]
fn imports_alone_dont_seed_false_positives() {
    // Two unrelated Rust files that happen to share several `use`
    // declarations must NOT be reported as duplicates. Pre-skip
    // behavior: the use-line tokens dominate small files and would
    // produce a fingerprint match.
    let a = r#"
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use thiserror::Error;

fn really_a_thing() -> i32 {
    42
}
"#;
    let b = r#"
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use thiserror::Error;

fn completely_different_logic(name: &str) -> String {
    format!("hi {}", name)
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        detect_opts(),
    );
    assert!(
        blocks.is_empty(),
        "shared imports must not seed a duplicate-block finding, got {blocks:?}"
    );
}

#[test]
fn ts_imports_are_skipped() {
    let a = r#"
import { foo, bar } from "./foo";
import * as React from "react";
import type { Baz } from "./baz";

function unique_a() {
    return 1;
}
"#;
    let b = r#"
import { foo, bar } from "./foo";
import * as React from "react";
import type { Baz } from "./baz";

function unique_b(x: number) {
    return x * 2;
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.ts".to_string(), a.to_string()),
            ("src/b.ts".to_string(), b.to_string()),
        ],
        detect_opts(),
    );
    assert!(
        blocks.is_empty(),
        "shared TS imports must not seed a finding, got {blocks:?}"
    );
}

#[test]
fn python_imports_are_skipped() {
    let a = r#"
import os
import sys
from collections import defaultdict, OrderedDict
from typing import List, Dict

def unique_a():
    return 1
"#;
    let b = r#"
import os
import sys
from collections import defaultdict, OrderedDict
from typing import List, Dict

def unique_b(x):
    return x * 2
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.py".to_string(), a.to_string()),
            ("src/b.py".to_string(), b.to_string()),
        ],
        detect_opts(),
    );
    assert!(
        blocks.is_empty(),
        "shared Python imports must not seed a finding, got {blocks:?}"
    );
}

#[test]
fn top_level_style_objects_are_ignored() {
    // Two unrelated TS files whose ONLY content is top-level
    // const-style declarations. These are AST-isomorphic with the
    // old token detector and used to cross-match. Function-anchored
    // detection ignores them — they're not inside a function body.
    let a = r#"
const chipStyle = {
    fontFamily: "ui-monospace, monospace",
    color: "var(--text-muted)",
    border: "1px solid var(--border-subtle)",
    borderRadius: 3,
    padding: "0 6px",
};

const labelStyle = {
    fontFamily: "ui-monospace, monospace",
    color: "var(--text-primary)",
};
"#;
    let b = r#"
const footerStyle = {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    padding: "4px 10px",
};

const bannerStyle = {
    display: "flex",
    color: "var(--accent)",
};
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.tsx".to_string(), a.to_string()),
            ("src/b.tsx".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(
        blocks.is_empty(),
        "top-level style objects must not surface as duplicates, got {blocks:?}"
    );
}

#[test]
fn top_level_error_enums_are_ignored() {
    // The case from commit 7feb819 — two thiserror-style enums in
    // different files would cross-match because their AST shapes
    // (derive attribute + variant + #[error("...")] + tuple-struct
    // String) are identical. These live at top level, not inside a
    // function, so the function-anchored detector skips them.
    let a = r#"
#[derive(Debug, thiserror::Error)]
pub enum CodeQualityError {
    #[error("scan task failed: {0}")]
    Task(String),
    #[error("scan timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("tree source failed: {0}")]
    TreeSource(String),
}
"#;
    let b = r#"
#[derive(Debug, thiserror::Error)]
pub enum LspClientError {
    #[error("no language server configured for `{0}`")]
    NoConfig(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("client not found: {0}")]
    NotFound(String),
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(
        blocks.is_empty(),
        "top-level error enums must not surface as duplicates, got {blocks:?}"
    );
}

#[test]
fn react_components_with_shared_idiom_but_different_logic_dont_match() {
    // Both functions use useRef + useEffect; one paints a div red,
    // one scrolls to top. Same idiom skeleton, completely different
    // body. This was an APTED false positive at 0.97 in the
    // similarity-rs spike. The subtree-hash approach treats the
    // ENTIRE function body as one hash; one differing string
    // assignment doesn't matter, but the called methods (style vs
    // scrollTop) and the parameter list shape differ enough to
    // produce different hashes once normalization preserves
    // structural punctuation.
    //
    // We're checking a behavior: small bodies with ID/STR
    // normalization may still hash-collide. If they do, the test
    // tells us that and we can iterate. For now: if it produces a
    // finding, the line span is short and falls below min_lines.
    let a = r#"
import { useEffect, useRef } from "react";

export function MountA(): number {
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!ref.current) return;
        ref.current.style.background = "red";
    }, []);
    return 1;
}
"#;
    let b = r#"
import { useEffect, useRef } from "react";

export function MountB(): string {
    const ref = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!ref.current) return;
        ref.current.scrollTop = 0;
    }, []);
    return "ok";
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.tsx".to_string(), a.to_string()),
            ("src/b.tsx".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    // Whatever surfaces, line span must be at least min_lines (10).
    // If both functions are ~9 lines, no finding will surface and
    // that's fine — the threshold is doing its job. Print for
    // visibility.
    println!("[react idiom] blocks: {blocks:?}");
    for b in &blocks {
        assert!(
            b.line_count >= DupOptions::default().min_lines,
            "min_lines threshold violated: {b:?}"
        );
    }
}

#[test]
fn whole_function_clone_subsumes_inner_block_clone() {
    // When two functions are identical in their entirety, we should
    // emit ONE finding spanning both function bodies — not also the
    // inner loop and the inner branch as separate findings.
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), body.to_string()),
            ("src/b.rs".to_string(), body.to_string()),
        ],
        detect_opts(),
    );
    let cross: Vec<_> = blocks.iter().filter(|b| b.a_path != b.b_path).collect();
    assert_eq!(
        cross.len(),
        1,
        "expected exactly 1 cross-file finding (whole-function), got {cross:?}",
    );
}

#[test]
fn skips_unsupported_languages() {
    let blocks = detect_duplicates(
        vec![
            (
                "README.md".to_string(),
                "# heading\nlots of text".to_string(),
            ),
            (
                "OTHER.md".to_string(),
                "# heading\nlots of text".to_string(),
            ),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}

#[test]
fn scoped_detects_clone_when_only_one_side_is_in_scope() {
    // Simulates the change-analysis flow: `src/a.rs` is the changed
    // file, `src/b.rs` is an unchanged peer. The scan must catch
    // the duplication even though only `a.rs` is in scope.
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    let mut scope = BTreeSet::new();
    scope.insert("src/a.rs".to_string());
    let blocks = detect_duplicates_scoped(
        vec![
            ("src/a.rs".to_string(), body.to_string()),
            ("src/b.rs".to_string(), body.to_string()),
            ("src/c.rs".to_string(), body.to_string()),
        ],
        &scope,
        detect_opts(),
    );
    assert!(!blocks.is_empty(), "expected at least one scoped match");
    // Every reported block must touch a scope path.
    for b in &blocks {
        assert!(
            b.a_path == "src/a.rs" || b.b_path == "src/a.rs",
            "scope filter violated: {b:?}",
        );
        // Side A is the scope side per the runner's convention.
        assert_eq!(b.a_path, "src/a.rs");
        assert_ne!(b.a_path, b.b_path, "same-file pair leaked: {b:?}");
    }
}

#[test]
fn scoped_drops_pairs_where_neither_side_is_in_scope() {
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    let mut scope = BTreeSet::new();
    scope.insert("src/changed.rs".to_string()); // not in corpus
    let blocks = detect_duplicates_scoped(
        vec![
            ("src/a.rs".to_string(), body.to_string()),
            ("src/b.rs".to_string(), body.to_string()),
        ],
        &scope,
        DupOptions::default(),
    );
    assert!(
        blocks.is_empty(),
        "expected no findings when scope doesn't intersect corpus, got {blocks:?}"
    );
}

#[test]
fn scoped_drops_same_file_self_match() {
    // A file containing two long, near-identical regions would
    // otherwise surface as an in-file dup. Scoped semantics drop it.
    let body_with_repeat = r#"
fn case_a(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}

fn case_b(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 { out.push(item * 2); }
        else if item < 0 { out.push(item * -1); }
        else { out.push(0); }
    }
    out
}
"#;
    let mut scope = BTreeSet::new();
    scope.insert("src/single.rs".to_string());
    let blocks = detect_duplicates_scoped(
        vec![("src/single.rs".to_string(), body_with_repeat.to_string())],
        &scope,
        DupOptions::default(),
    );
    for b in &blocks {
        assert_ne!(b.a_path, b.b_path, "same-file pair leaked: {b:?}");
    }
}

#[test]
fn nested_inline_functions_dont_re_stamp_subtrees_via_ancestors() {
    // Two TS files where one of them contains nested inline arrow
    // functions sharing a common subtree shape with helpers in the
    // other file. Pre-fix, the inner subtree was stamped once per
    // ancestor function, so pair_up emitted one DuplicateBlock per
    // (outer_fn, inner_fn) ancestry combination — same line-range
    // rows repeating dozens of times. With the nested-function gate
    // each subtree is recorded under exactly one FnId and the
    // duplicate count stays bounded.
    let host = r#"
function App() {
    return [
        () => {
            const items = [1, 2, 3];
            for (const item of items) {
                if (item > 0) {
                    console.log(item);
                }
            }
        },
        () => {
            const items = [4, 5, 6];
            for (const item of items) {
                if (item > 0) {
                    console.log(item);
                }
            }
        },
        () => {
            const items = [7, 8, 9];
            for (const item of items) {
                if (item > 0) {
                    console.log(item);
                }
            }
        },
    ];
}
"#;
    let peer = r#"
function elsewhere() {
    const items = [1, 2, 3];
    for (const item of items) {
        if (item > 0) {
            console.log(item);
        }
    }
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/host.ts".to_string(), host.to_string()),
            ("src/peer.ts".to_string(), peer.to_string()),
        ],
        detect_opts(),
    );
    // Three inner arrows × the peer function = 3 cross-file pairs at
    // most (one DuplicateBlock per ordered fn pair). Pre-fix this
    // emitted ~12 because each outer ancestor (App + each arrow's
    // closure path through siblings) stamped the same subtree.
    let cross: Vec<_> = blocks.iter().filter(|b| b.a_path != b.b_path).collect();
    assert!(
        cross.len() <= 3,
        "expected at most 3 cross-file blocks (one per inner arrow ↔ peer fn pair), got {}: {cross:#?}",
        cross.len(),
    );
}

#[test]
fn clojure_clone_across_two_files() {
    let body = r#"
(ns foo.a
  (:require [clojure.string :as str]))

(defn process [items]
  (let [out (atom [])]
    (doseq [item items]
      (cond
        (pos? item) (swap! out conj (* item 2))
        (neg? item) (swap! out conj (* item -1))
        :else (swap! out conj 0)))
    @out))
"#;
    let other = r#"
(ns foo.b
  (:require [clojure.string :as str]))

(defn process [items]
  (let [out (atom [])]
    (doseq [item items]
      (cond
        (pos? item) (swap! out conj (* item 2))
        (neg? item) (swap! out conj (* item -1))
        :else (swap! out conj 0)))
    @out))
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.clj".to_string(), body.to_string()),
            ("src/b.clj".to_string(), other.to_string()),
        ],
        detect_opts(),
    );
    assert!(
        !blocks.is_empty(),
        "expected Clojure defn clone to be detected"
    );
}

#[test]
fn clojure_ns_require_preamble_is_skipped() {
    let a = r#"
(ns foo.a
  (:require [clojure.string :as str]
            [clojure.set :as set])
  (:import [java.util Date UUID]))

(defn unique-a []
  1)
"#;
    let b = r#"
(ns foo.b
  (:require [clojure.string :as str]
            [clojure.set :as set])
  (:import [java.util Date UUID]))

(defn unique-b [x]
  (* x 2))
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.clj".to_string(), a.to_string()),
            ("src/b.clj".to_string(), b.to_string()),
        ],
        detect_opts(),
    );
    assert!(
        blocks.is_empty(),
        "shared Clojure ns/require/import preamble must not seed a finding, got {blocks:?}"
    );
}

/// Detection parses the whole corpus and is CPU-bound, so runs never
/// overlap within a process, whoever asks: a change's scan and the
/// whole-tree collector's wait for each other instead of competing for
/// the same cores.
#[test]
fn detections_run_one_at_a_time() {
    let body = "fn f(x: i32) -> i32 {\n    let a = x + 1;\n    let b = a * 2;\n    let c = b - 3;\n    let d = c / 4;\n    a + b + c + d\n}\n";
    let corpus: Vec<(String, String)> = (0..300)
        .map(|i| (format!("src/f{i}.rs"), body.to_string()))
        .collect();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let corpus = corpus.clone();
            std::thread::spawn(move || detect_duplicates(corpus, detect_opts()))
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(peak_running(), 1);
}

/// `n` files each holding the same function: a duplicate-dense corpus.
fn dense(n: usize) -> Vec<(String, String)> {
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    (0..n)
        .map(|i| (format!("src/m{i:03}.rs"), body.to_string()))
        .collect()
}

/// What a detection reports grows with the corpus, not with its square: n
/// identical functions are n regions, each linked to a peer, not the
/// n(n-1)/2 pairs of them. (A dense corpus made the pair list the largest
/// thing in memory.)
#[test]
fn a_dense_clone_family_reports_a_pair_per_member_not_per_pair_of_members() {
    let n = 60;
    let blocks = detect_duplicates(dense(n), detect_opts());
    assert!(
        blocks.len() < 2 * n,
        "{} blocks for {n} identical functions",
        blocks.len()
    );
    // Every member is still reported, on one side or the other.
    for i in 0..n {
        let path = format!("src/m{i:03}.rs");
        assert!(
            blocks.iter().any(|b| b.a_path == path || b.b_path == path),
            "{path} is not in any block"
        );
    }
}

/// A changed file in a big family still sees its clone.
#[test]
fn a_dense_family_member_in_scope_still_has_a_finding() {
    let scope: BTreeSet<String> = ["src/m037.rs".to_string()].into();
    let blocks = detect_duplicates_scoped(dense(60), &scope, detect_opts());
    assert!(!blocks.is_empty());
    assert!(blocks.iter().all(|b| b.a_path == "src/m037.rs"));
}

/// Two identical blocks in one function and a third elsewhere: each
/// function is paired, whichever the group's first member is in.
#[test]
fn members_in_the_firsts_own_function_are_paired_with_another_function() {
    let twice = r#"
fn both(items: Vec<i32>) -> i32 {
    let mut total = 0;
    for item in &items {
        if *item > 0 {
            total += item * 2;
        } else if *item < 0 {
            total -= item * 3;
        } else {
            total += 1;
        }
    }
    for item in &items {
        if *item > 0 {
            total += item * 2;
        } else if *item < 0 {
            total -= item * 3;
        } else {
            total += 1;
        }
    }
    total
}
"#;
    let once = r#"
fn single(items: Vec<i32>) -> i32 {
    let mut total = 0;
    for item in &items {
        if *item > 0 {
            total += item * 2;
        } else if *item < 0 {
            total -= item * 3;
        } else {
            total += 1;
        }
    }
    total
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), twice.to_string()),
            ("src/b.rs".to_string(), once.to_string()),
        ],
        detect_opts(),
    );
    assert!(blocks
        .iter()
        .any(|b| b.a_path == "src/a.rs" && b.b_path == "src/b.rs"));
}

/// A function nothing else in the corpus resembles: its body is shaped by
/// `salt` (loops nested `salt` deep), so no two share a hash.
fn unique_file(salt: usize) -> (String, String) {
    let mut body = String::from("fn f(items: &[i32]) -> i32 {\n    let mut t = 0;\n");
    for d in 0..=salt {
        body.push_str(&format!("{}for x{d} in items {{\n", "    ".repeat(d + 1)));
    }
    body.push_str(&format!("{}t += 1;\n", "    ".repeat(salt + 2)));
    for d in (0..=salt).rev() {
        body.push_str(&format!("{}}}\n", "    ".repeat(d + 1)));
    }
    body.push_str("    if t > 0 { t } else { -t }\n}\n");
    (format!("src/u{salt:03}.rs"), body)
}

/// A scoped scan keeps the subtrees of the files in scope and, of the rest,
/// only those that match one of theirs — not every subtree in the tree
/// (what a one-file change paid for in memory and in pairing).
#[test]
fn a_scoped_scan_keeps_only_subtrees_that_could_match_the_scope() {
    let clone = dense(1).remove(0).1;
    let mut files: Vec<(String, String)> = (0..40).map(unique_file).collect();
    files.push(("src/scope.rs".into(), clone.clone()));
    files.push(("src/twin.rs".into(), clone));
    let scope: BTreeSet<String> = ["src/scope.rs".to_string()].into();
    let blocks = detect_duplicates_scoped(files.clone(), &scope, detect_opts());
    assert!(
        blocks
            .iter()
            .any(|b| b.a_path == "src/scope.rs" && b.b_path == "src/twin.rs"),
        "{blocks:?}"
    );
    let kept = last_scan_records();
    let whole = {
        detect_duplicates(files, detect_opts());
        last_scan_records()
    };
    assert!(kept * 4 < whole, "kept {kept} of {whole} subtrees");
}

/// What it reports is what a whole-tree scan filtered to the scope reports.
#[test]
fn a_scoped_scan_reports_what_the_whole_tree_scan_would_for_the_scope() {
    let mut files = dense(6);
    files.extend((0..10).map(unique_file));
    let scope: BTreeSet<String> = ["src/m002.rs".to_string(), "src/u003.rs".to_string()].into();
    let scoped = detect_duplicates_scoped(files.clone(), &scope, detect_opts());
    assert!(scoped.iter().any(|b| b.a_path == "src/m002.rs"));
    assert!(scoped.iter().all(|b| scope.contains(&b.a_path)));
    // Whole tree, then filtered to the scope: the same regions.
    let regions = |blocks: &[DuplicateBlock]| -> BTreeSet<(String, u32, u32)> {
        blocks
            .iter()
            .flat_map(|b| {
                [
                    (b.a_path.clone(), b.a_start_line, b.a_end_line),
                    (b.b_path.clone(), b.b_start_line, b.b_end_line),
                ]
            })
            .filter(|(p, _, _)| scope.contains(p))
            .collect()
    };
    let whole: Vec<DuplicateBlock> = detect_duplicates(files, detect_opts())
        .into_iter()
        .filter(|b| b.a_path != b.b_path)
        .collect();
    assert_eq!(regions(&scoped), regions(&whole));
}

fn cached(files: &[(String, String)], with_keys: bool) -> Vec<CachedFile> {
    files
        .iter()
        .map(|(p, t)| {
            let text = t.clone();
            CachedFile {
                path: p.clone(),
                key: with_keys.then(|| FileKey {
                    space: "test".into(),
                    id: format!("{p}:{}", text.len()),
                }),
                text: Box::new(move || Some(text)),
            }
        })
        .collect()
}

fn family() -> (Vec<(String, String)>, BTreeSet<String>) {
    let clone = dense(1).remove(0).1;
    let mut files: Vec<(String, String)> = (0..12).map(unique_file).collect();
    files.push(("src/scope.rs".into(), clone.clone()));
    files.push(("src/twin.rs".into(), clone));
    (files, ["src/scope.rs".to_string()].into())
}

/// A second scan of the same tree parses nothing and says so; the answer
/// is the uncached scoped scan's.
#[test]
fn a_second_scan_of_an_unchanged_tree_parses_nothing() {
    let (files, scope) = family();
    let cache = DupCache::default();
    let (first, cold) =
        detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    assert_eq!(cold.hits, 0);
    assert_eq!(cold.misses, files.len());
    let parsed = parses();
    let (second, warm) =
        detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    assert_eq!(parses(), parsed, "the warm scan parsed");
    assert_eq!((warm.hits, warm.misses), (files.len(), 0));
    assert_eq!(first, second);
    assert_eq!(
        first,
        detect_duplicates_scoped(files.clone(), &scope, detect_opts())
    );
}

/// A cached file's text is never loaded.
#[test]
fn a_cached_file_is_not_read() {
    let (files, scope) = family();
    let cache = DupCache::default();
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    let unread: Vec<CachedFile> = files
        .iter()
        .map(|(p, t)| CachedFile {
            path: p.clone(),
            key: Some(FileKey {
                space: "test".into(),
                id: format!("{p}:{}", t.len()),
            }),
            text: Box::new(|| panic!("read a cached file")),
        })
        .collect();
    detect_duplicates_scoped_cached(unread, &scope, detect_opts(), &cache);
}

/// An edit is a new content id: that file alone is parsed again.
#[test]
fn an_edited_file_is_parsed_again_and_no_other() {
    let (mut files, scope) = family();
    let cache = DupCache::default();
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    files[3].1.push_str("\n// edited\n");
    let (_, stats) =
        detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    assert_eq!((stats.hits, stats.misses), (files.len() - 1, 1));
}

/// Different floors are different records.
#[test]
fn the_size_floors_are_part_of_the_key() {
    let (files, scope) = family();
    let cache = DupCache::default();
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &cache);
    let other = DupOptions {
        min_nodes: 60,
        ..detect_opts()
    };
    let (_, stats) = detect_duplicates_scoped_cached(cached(&files, true), &scope, other, &cache);
    assert_eq!(stats.hits, 0);
}

/// Files without a content id are parsed every time and never cached.
#[test]
fn a_file_with_no_key_bypasses_the_cache() {
    let (files, scope) = family();
    let cache = DupCache::default();
    let (_, stats) =
        detect_duplicates_scoped_cached(cached(&files, false), &scope, detect_opts(), &cache);
    assert_eq!((stats.hits, stats.misses), (0, files.len()));
    assert_eq!(cache.held(), (0, 0));
}

/// The cache stays inside its byte budget, dropping whole files, oldest
/// first.
#[test]
fn the_cache_evicts_the_least_recently_used_file_to_stay_in_budget() {
    let (files, scope) = family();
    let whole = DupCache::default();
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &whole);
    let (records, bytes) = whole.held();
    assert!(records > 0 && bytes == records * RECORD_BYTES);

    let small = DupCache::with_budget(bytes / 3);
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &small);
    let (_, held) = small.held();
    assert!(held <= bytes / 3, "{held} > {}", bytes / 3);
    assert!(held > 0);
    // A budget below one file's records holds nothing.
    let tiny = DupCache::with_budget(RECORD_BYTES);
    detect_duplicates_scoped_cached(cached(&files, true), &scope, detect_opts(), &tiny);
    assert_eq!(tiny.held().1, 0);
}

/// The records a fixed source produces, pinned to `DETECTOR_VERSION`:
/// changing how subtrees are hashed or chosen fails this until the version
/// is bumped and the expectation re-pinned, so a cache can't serve records
/// the current detector wouldn't make.
#[test]
fn the_records_are_pinned_to_the_detector_version() {
    let body = dense(1).remove(0).1;
    let mut out = Vec::new();
    scan_file("src/pin.rs", &body, detect_opts(), None, &mut out);
    let hashes: Vec<(u64, u32, u32, u32)> = out
        .iter()
        .map(|s| (s.hash, s.node_count, s.start_line, s.end_line))
        .collect();
    assert_eq!(DETECTOR_VERSION, 1, "re-pin the hashes below, then bump");
    assert_eq!(hashes, PINNED_V1);
}

const PINNED_V1: &[(u64, u32, u32, u32)] = &[
    (3679152524243699532, 116, 2, 14),
    (13672393085675031171, 94, 2, 14),
    (3199666752499502949, 76, 4, 12),
    (362106070147055594, 75, 4, 12),
    (7382068709588596213, 70, 4, 12),
    (16313548889648378043, 67, 5, 11),
    (3996386052852592696, 66, 5, 11),
    (12028981030681832557, 43, 7, 11),
    (378789080419906373, 41, 7, 11),
];
