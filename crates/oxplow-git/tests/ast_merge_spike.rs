//! Tier-2 AST-merge **spike** (tsk121) — NOT production code.
//!
//! Proves the risky core of a tree-sitter structural 3-way merge is
//! tractable on oxplow's *existing* parser deps, and that the safety
//! model (never auto-resolve a true semantic overlap) falls out of a
//! per-top-level-item classify. This is the build-our-own path; Mergiraf
//! stays out of the link graph (GPLv3 vs our MIT — see
//! `.context/smart-merge.md`).
//!
//! Scope of the spike, deliberately tiny:
//!   * one language (Rust), parsed via `tree_sitter_rust`;
//!   * top-level items only (`use`, `fn`, `struct`, …) — exactly the
//!     "commutative parent" cases Tier-2 targets first;
//!   * identity = (kind, name) for named items, full text for `use`;
//!   * a classic per-item 3-way classify, refusing when both sides edit
//!     the *same* item differently.
//!
//! What it deliberately does NOT do (these are the follow-up build): line
//! anchoring / order preservation via PCS, sub-item merge, the other 8
//! languages, parse-error fallback, git-index wiring. Order here is
//! normalized to "base order, then new ours, then new theirs" — good
//! enough to prove feasibility, not the shipping ordering policy.

use std::collections::BTreeMap;

use tree_sitter::{Node, Parser};

/// A top-level declaration, reduced to the two things the structural
/// merge needs: a stable identity `key` and the verbatim source `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    key: String,
    text: String,
}

/// The result of merging one item-set against another over a base.
#[derive(Debug, PartialEq, Eq)]
enum Merge {
    Clean(Vec<Item>),
    /// At least one item was edited incompatibly by both sides — a true
    /// semantic overlap. The real driver leaves git's markers here.
    Conflict(Vec<String>),
}

/// Parse Rust source into its ordered top-level items.
///
/// Identity rules (the spike's whole heuristic): a `use_declaration` is
/// identified by its normalized text (a use *is* what it imports); every
/// other named top-level item is identified by `kind + name`, so editing
/// a function body is seen as "same item, changed", not add+remove.
fn top_level_items(src: &str) -> Vec<Item> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("load rust grammar");
    let tree = parser.parse(src, None).expect("parse");
    let root = tree.root_node();

    let mut out = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.is_extra() {
            continue; // comments / whitespace at top level
        }
        let text = node_text(child, src);
        let key = item_key(child, src, &text);
        out.push(Item { key, text });
    }
    out
}

fn node_text(node: Node, src: &str) -> String {
    node.utf8_text(src.as_bytes())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn item_key(node: Node, src: &str, text: &str) -> String {
    let kind = node.kind();
    if kind == "use_declaration" {
        // Normalize interior whitespace so `use a::b ;` == `use a::b;`.
        return format!(
            "use::{}",
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
    if let Some(name) = node.child_by_field_name("name") {
        return format!("{kind}::{}", node_text(name, src));
    }
    // Unnamed top-level item (e.g. a bare `impl`): fall back to its text
    // so identical blocks match and divergent ones are seen as distinct.
    format!("{kind}::{text}")
}

/// Structural 3-way merge over top-level items.
///
/// Per key, classify exactly like a token diff3 but at item granularity:
/// take the side that changed, take agreement, and refuse only when both
/// sides changed the *same* key to *different* text. Added-by-one-side
/// items (the import / decl commutative cases) merge unconditionally;
/// added-by-both-with-different-text under one key is the lone add/add
/// conflict.
fn merge_items(base: &[Item], ours: &[Item], theirs: &[Item]) -> Merge {
    let bmap = index(base);
    let omap = index(ours);
    let tmap = index(theirs);

    let mut keys: Vec<&String> = bmap.keys().chain(omap.keys()).chain(tmap.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut merged: Vec<Item> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    for key in keys {
        let b = bmap.get(key);
        let o = omap.get(key);
        let t = tmap.get(key);
        match (b, o, t) {
            // Present everywhere: classic 3-way on the item text.
            (Some(b), Some(o), Some(t)) => {
                if o.text == t.text {
                    merged.push((*o).clone());
                } else if o.text == b.text {
                    merged.push((*t).clone());
                } else if t.text == b.text {
                    merged.push((*o).clone());
                } else {
                    conflicts.push((*key).clone());
                }
            }
            // Deleted by both, or by one with the other unchanged: drop.
            (Some(b), None, Some(t)) if t.text == b.text => {}
            (Some(b), Some(o), None) if o.text == b.text => {}
            // Delete vs edit — true overlap.
            (Some(_), None, Some(_)) | (Some(_), Some(_), None) => conflicts.push((*key).clone()),
            (Some(_), None, None) => {}
            // Added by one side only — the commutative win.
            (None, Some(o), None) => merged.push((*o).clone()),
            (None, None, Some(t)) => merged.push((*t).clone()),
            // Added by both: fine iff identical, else add/add conflict.
            (None, Some(o), Some(t)) => {
                if o.text == t.text {
                    merged.push((*o).clone());
                } else {
                    conflicts.push((*key).clone());
                }
            }
            (None, None, None) => unreachable!("key came from some map"),
        }
    }

    if conflicts.is_empty() {
        Merge::Clean(merged)
    } else {
        Merge::Conflict(conflicts)
    }
}

fn index(items: &[Item]) -> BTreeMap<String, &Item> {
    items.iter().map(|i| (i.key.clone(), i)).collect()
}

fn merge_src(base: &str, ours: &str, theirs: &str) -> Merge {
    merge_items(
        &top_level_items(base),
        &top_level_items(ours),
        &top_level_items(theirs),
    )
}

fn clean_keys(m: &Merge) -> Vec<String> {
    match m {
        Merge::Clean(items) => items.iter().map(|i| i.key.clone()).collect(),
        Merge::Conflict(c) => panic!("expected clean merge, got conflict on {c:?}"),
    }
}

// --- Feasibility proofs ---------------------------------------------------

#[test]
fn parses_top_level_items_via_existing_treesitter_dep() {
    let items = top_level_items("use std::fmt;\nfn a() {}\nstruct S;\n");
    let keys: Vec<_> = items.iter().map(|i| i.key.as_str()).collect();
    assert!(keys.contains(&"use::use std::fmt;"));
    assert!(keys.iter().any(|k| k.starts_with("function_item::a")));
    assert!(keys.iter().any(|k| k.starts_with("struct_item::S")));
}

#[test]
fn both_add_different_imports_merges_structurally() {
    // The flagship Tier-2 case: two new `use`s, no textual adjacency
    // needed — identity is the import itself, so order is irrelevant.
    let base = "use std::fmt;\n";
    let ours = "use std::collections::HashMap;\nuse std::fmt;\n";
    let theirs = "use std::fmt;\nuse std::io;\n";
    let keys = clean_keys(&merge_src(base, ours, theirs));
    assert!(keys.iter().any(|k| k.contains("HashMap")));
    assert!(keys.iter().any(|k| k.contains("std::io")));
    assert!(keys.iter().any(|k| k.contains("std::fmt")));
}

#[test]
fn independently_added_top_level_fns_merge() {
    let base = "fn shared() {}\n";
    let ours = "fn shared() {}\nfn from_ours() {}\n";
    let theirs = "fn shared() {}\nfn from_theirs() {}\n";
    let keys = clean_keys(&merge_src(base, ours, theirs));
    assert!(keys
        .iter()
        .any(|k| k.starts_with("function_item::from_ours")));
    assert!(keys
        .iter()
        .any(|k| k.starts_with("function_item::from_theirs")));
    assert!(keys.iter().any(|k| k.starts_with("function_item::shared")));
}

#[test]
fn one_side_edits_a_body_other_adds_a_fn_merges() {
    // Reordering / unrelated edits across distinct items never collide.
    let base = "fn a() { 1 }\n";
    let ours = "fn a() { 2 }\n"; // body edit
    let theirs = "fn a() { 1 }\nfn b() {}\n"; // new fn
    let merged = merge_src(base, ours, theirs);
    let keys = clean_keys(&merged);
    assert!(keys.iter().any(|k| k.starts_with("function_item::b")));
    // The edited body wins (theirs left `a` at base).
    if let Merge::Clean(items) = &merged {
        let a = items
            .iter()
            .find(|i| i.key.starts_with("function_item::a"))
            .unwrap();
        assert!(a.text.contains("2"), "edited body should win: {:?}", a.text);
    }
}

#[test]
fn same_fn_edited_differently_is_a_semantic_overlap() {
    // SAFETY proof: both sides change the *same* item differently. The
    // structural merge must refuse — exactly the case we never silently
    // auto-resolve. Real driver leaves git's conflict markers.
    let base = "fn calc() { a() }\n";
    let ours = "fn calc() { b() }\n";
    let theirs = "fn calc() { c() }\n";
    match merge_src(base, ours, theirs) {
        Merge::Conflict(keys) => {
            assert!(keys.iter().any(|k| k.starts_with("function_item::calc")))
        }
        Merge::Clean(_) => panic!("must not auto-resolve a divergent same-item edit"),
    }
}

#[test]
fn both_add_same_named_fn_with_different_bodies_conflicts() {
    // add/add at one identity with different text — the lone add/add case.
    let base = "fn keep() {}\n";
    let ours = "fn keep() {}\nfn new() { 1 }\n";
    let theirs = "fn keep() {}\nfn new() { 2 }\n";
    assert!(matches!(merge_src(base, ours, theirs), Merge::Conflict(_)));
}
