//! Tier-2 AST structural merge — parse + per-item 3-way merge
//! (wired into auto-resolve in; second-slice
//! languages added in).
//!
//! The language-neutral core of the AST merge designed in
//! `.context/smart-merge.md` (Tier 2), in two layers:
//!
//! 1. **Parse → items** ([`parse_top_level_items`]): parse a
//!    file's bytes with the right tree-sitter grammar and reduce it to
//!    its ordered top-level items, each carrying
//!    - a **stable identity key** for 3-way matching (an import is keyed
//!      by its normalized text; a named declaration by `kind + name`;
//!      anything unnamed falls back to `kind + normalized-text`), and
//!    - its **original byte span** (so reconstruction reuses exact
//!      source, including a declaration's attached leading doc-comments).
//! 2. **3-way merge** ([`merge_top_level`]): per-item classify
//!    (take-the-changed-side / take-agreement / refuse on overlap),
//!    deterministic ordering + verbatim reconstruction, and a re-parse
//!    guard that discards any reconstruction that doesn't parse cleanly.
//!
//! Wired into `auto_resolve_conflicts` as a Tier-1.5 fallback in
//! (see `smart_merge.rs`). The original feasibility spike lives at
//! `crates/oxplow-git/tests/ast_merge_spike.rs`.
//!
//! ## Safety
//!
//! [`parse_top_level_items`] returns `None` when the file isn't a
//! supported language *or* the parse produced any error/missing node —
//! we never operate on an untrusted tree. Callers treat `None` as "leave
//! git's conflict markers", preserving Tier-2's reduce-only invariant.
//!
//! ## Supported languages
//!
//! First slice: rust, typescript, tsx, javascript, python, go.
//! Second slice: java, c, cpp, clojure. C/C++ and Clojure use
//! bespoke key resolvers (see [`KeyStrategy`]); the rest share the generic
//! `name_fields` path.
//!
//! ## Identity-key fidelity
//!
//! Imports and the high-value named declarations (`fn`/function, `struct`,
//! `class`/`interface`/`enum`, `type` alias, `trait`, Java type decls, …)
//! get precise `kind+name` keys, which is what the flagship commutative
//! cases need (both sides add a different import / a different top-level
//! decl). Go methods are disambiguated by their receiver; C/C++ functions
//! by their declarator (name + signature, so overloads stay distinct);
//! Clojure forms by their head symbol + defined name.
//!
//! Declarations whose name sits under a declarator list and that take the
//! `Generic` strategy (Rust/TS `const`/`let`/`var`, Go `type`/`var`/`const`
//! groups) fall back to text identity. That merges correctly when one side
//! *adds* such an item (add is key-independent), but because a text key
//! changes when its body changes, two sides editing the *same* such item
//! differently are mis-classified as two independent adds rather than a
//! conflict — see the `text-identity divergent-edit` note in
//! `.context/smart-merge.md` (tracked as a separate hardening task).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use oxplow_code_metrics::{MergeKey, MergeSpec};
use tree_sitter::Node;

pub use oxplow_code_metrics::{Language, Languages};

/// The language a path merges as: its language when that language has a
/// merge facet (`merge:` in its analysis.yaml), so a `.cs` file (no facet)
/// is `None` and keeps git's markers.
pub fn language_for_path<'a>(languages: &'a Languages, path: &str) -> Option<&'a Language> {
    languages
        .for_path(path)
        .filter(|l| l.spec().merge.is_some())
}

/// The merge facet of a language, when it has one.
pub fn merge_spec(lang: &Language) -> Option<&MergeSpec> {
    lang.analyzable()
        .then(|| lang.spec().merge.as_ref())
        .flatten()
}

/// A top-level declaration, reduced to what the structural merge needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable identity for 3-way matching across base/ours/theirs.
    pub key: String,
    /// Verbatim source slice covering `byte_span` (includes any attached
    /// leading doc-comments). Reconstruction splices this back unchanged.
    pub text: String,
    /// Byte range in the original source. `text == src[byte_span]`.
    pub byte_span: Range<usize>,
}

/// Parse `src` as `lang` and return its ordered top-level items, or
/// `None` if the parse failed (any error/missing node) — never operate
/// on an untrusted tree.
pub fn parse_top_level_items(src: &str, lang: &Language) -> Option<Vec<Item>> {
    let spec = merge_spec(lang)?;
    let mut parser = lang.parser()?;
    let tree = parser.parse(src, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut items = Vec::new();
    let mut cursor = root.walk();
    // Track a contiguous run of leading comments to attach to the next
    // declaration (so a moved decl carries its doc comment).
    let mut comment_run: Option<(usize, usize)> = None; // (start_byte, end_byte)

    for child in root.children(&mut cursor) {
        if child.is_extra() {
            // Comment / whitespace at top level. Extend (or open) the run.
            let start = comment_run.map_or(child.start_byte(), |(s, _)| s);
            comment_run = Some((start, child.end_byte()));
            continue;
        }

        // Absorb the preceding comment run into this item's span iff it
        // is directly attached (no blank line between it and the decl).
        // A DETACHED run (blank line between) becomes its own item instead of
        // being dropped — a licence header or section divider is content, and
        // silently losing it while reporting a clean merge is the worst
        // outcome available.
        let mut start = child.start_byte();
        if let Some((cstart, cend)) = comment_run {
            let gap = &src[cend..child.start_byte()];
            if gap.contains("\n\n") {
                push_comment_item(&mut items, src, cstart, cend);
            } else {
                start = cstart;
            }
        }
        comment_run = None;

        let byte_span = start..child.end_byte();
        let text = src[byte_span.clone()].to_string();
        let key = item_key(child, src, spec, lang.spec().forms.as_ref());
        items.push(Item {
            key,
            text,
            byte_span,
        });
    }

    // A run trailing the last declaration (or a comment-only file) attaches to
    // nothing, so it needs the same treatment.
    if let Some((cstart, cend)) = comment_run {
        push_comment_item(&mut items, src, cstart, cend);
    }

    Some(items)
}

/// Emit a detached comment run as a standalone [`Item`].
///
/// Keyed by normalized text, like imports: an identical header on all three
/// sides matches and survives once, while a header edited on one side reads as
/// a delete + add and lands in the conservative conflict path rather than being
/// silently rewritten. Whitespace-only runs are skipped — `is_extra()` covers
/// whitespace as well as comments, and a blank-line "item" is not content.
fn push_comment_item(items: &mut Vec<Item>, src: &str, start: usize, end: usize) {
    let text = &src[start..end];
    if text.trim().is_empty() {
        return;
    }
    items.push(Item {
        key: format!("{COMMENT_KEY_PREFIX}{}", normalize_ws(text)),
        text: text.to_string(),
        byte_span: start..end,
    });
}

/// Key prefix marking a standalone comment block. `reconstruct` reads
/// it to restore the blank line that made the block detached in the first
/// place.
const COMMENT_KEY_PREFIX: &str = "#comment::";

/// Compute an item's identity key per the spec's rules.
fn item_key(
    node: Node,
    src: &str,
    spec: &MergeSpec,
    forms: Option<&oxplow_code_metrics::Forms>,
) -> String {
    let kind = node.kind();

    // Imports/use: identity is the normalized text (all strategies).
    if spec.import_kinds.iter().any(|k| k == kind) {
        return format!("import::{}", normalize_ws(&node_text(node, src)));
    }

    match spec.key {
        MergeKey::Named => generic_item_key(node, src, spec),
        MergeKey::CDeclarator => c_family_item_key(node, src),
        MergeKey::FormHead => clojure_item_key(node, src, spec, forms),
    }
}

/// The default key: see-through wrappers + `name_fields` + Go-receiver
/// disambiguation, falling back to `kind + normalized-text`.
fn generic_item_key(node: Node, src: &str, spec: &MergeSpec) -> String {
    let kind = node.kind();

    // See through wrappers (export/decorated) to the inner declaration
    // for naming, but key off the *inner* kind+name.
    let named = if spec.transparent_kinds.iter().any(|k| k == kind) {
        inner_decl(node, spec).unwrap_or(node)
    } else {
        node
    };

    if let Some(name) = decl_name(named, src, spec) {
        let inner_kind = named.kind();
        // Disambiguate Go methods by receiver type — two methods can
        // share a name on different receivers.
        if let Some(recv) = named.child_by_field_name("receiver") {
            return format!(
                "{inner_kind}::{}::{name}",
                normalize_ws(&node_text(recv, src))
            );
        }
        return format!("{inner_kind}::{name}");
    }

    // Unnamed item (bare `impl`, const/var declarator lists, …): fall
    // back to kind + normalized text so identical blocks match and
    // divergent ones stay distinct.
    format!("{kind}::{}", normalize_ws(&node_text(node, src)))
}

/// C/C++ identity key. Functions key by their declarator (name +
/// signature, so overloads stay distinct); structs/unions/enums/classes/
/// namespaces by name; typedefs and globals by their declarator's leaf
/// identifier. `template_declaration` is seen through to the wrapped
/// declaration. Anything else → `kind + normalized-text`.
fn c_family_item_key(node: Node, src: &str) -> String {
    // See through a `template<…>` wrapper to the declaration it templates.
    let inner = if node.kind() == "template_declaration" {
        c_template_inner(node).unwrap_or(node)
    } else {
        node
    };
    let kind = inner.kind();
    match kind {
        // Name + signature: the declarator text covers both, and is stable
        // across body edits while keeping overloads distinct.
        "function_definition" => {
            if let Some(d) = inner.child_by_field_name("declarator") {
                return format!("function_definition::{}", normalize_ws(&node_text(d, src)));
            }
        }
        "struct_specifier"
        | "union_specifier"
        | "enum_specifier"
        | "class_specifier"
        | "namespace_definition" => {
            if let Some(n) = inner.child_by_field_name("name") {
                return format!("{kind}::{}", node_text(n, src));
            }
        }
        // typedef alias name / global variable name: the declarator's leaf
        // identifier (a value edit must not change identity).
        "type_definition" | "declaration" => {
            if let Some(name) = c_declarator_leaf_name(inner) {
                return format!("{kind}::{}", node_text(name, src));
            }
        }
        _ => {}
    }
    format!("{}::{}", node.kind(), normalize_ws(&node_text(node, src)))
}

/// First `function_definition`/`*_specifier`/`declaration` child of a C++
/// `template_declaration` (the declaration it templates).
fn c_template_inner(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| {
        matches!(
            c.kind(),
            "function_definition"
                | "struct_specifier"
                | "union_specifier"
                | "enum_specifier"
                | "class_specifier"
                | "declaration"
        )
    });
    found
}

/// Descend a node's `declarator` field chain to its leaf identifier.
fn c_declarator_leaf_name(node: Node) -> Option<Node> {
    let mut cur = node.child_by_field_name("declarator")?;
    loop {
        match cur.kind() {
            "identifier" | "type_identifier" | "field_identifier" | "qualified_identifier" => {
                return Some(cur)
            }
            _ => cur = cur.child_by_field_name("declarator")?,
        }
    }
}

/// Clojure identity key. A top-level `(head name …)` form whose head is a
/// recognized `def…`/`ns` symbol keys by `head::name`; any other form by
/// `kind + normalized-text`. Two `defmethod`s on the same multifn collapse
/// to one key and conservatively bail (DuplicateKeys) — safe, not wrong.
fn clojure_item_key(
    node: Node,
    src: &str,
    spec: &MergeSpec,
    forms: Option<&oxplow_code_metrics::Forms>,
) -> String {
    if forms.is_some_and(|f| node.kind() == f.list) {
        let mut cursor = node.walk();
        let values: Vec<Node> = node.children_by_field_name("value", &mut cursor).collect();
        if let (Some(head), Some(name)) = (values.first(), values.get(1)) {
            let head_txt = node_text(*head, src);
            if spec.def_heads.contains(&head_txt) {
                return format!("{head_txt}::{}", normalize_ws(&node_text(*name, src)));
            }
        }
    }
    format!("{}::{}", node.kind(), normalize_ws(&node_text(node, src)))
}

/// First child reachable through the spec's `inner_decl_fields`.
fn inner_decl<'a>(node: Node<'a>, spec: &MergeSpec) -> Option<Node<'a>> {
    spec.inner_decl_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
}

/// A declaration's name via the spec's `name_fields`, if present.
fn decl_name(node: Node, src: &str, spec: &MergeSpec) -> Option<String> {
    spec.name_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .map(|n| node_text(n, src))
}

fn node_text(node: Node, src: &str) -> String {
    node.utf8_text(src.as_bytes())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Collapse interior whitespace runs to single spaces so trivial
/// reformatting (`use a::b ;` vs `use a::b;`) keys identically.
fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ===================================================================
// Per-item 3-way merge: classify + ordering/reconstruction + re-parse
// guard. Built on `parse_top_level_items`. Pure; no merge is wired into
// `auto_resolve_conflicts` yet (that's).
// ===================================================================

/// Outcome of a top-level structural 3-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstMerge {
    /// Every item was independently resolvable. The `String` is the
    /// reconstructed, re-parse-validated file content.
    Resolved(String),
    /// At least one item is a true semantic overlap (a divergent same-key
    /// edit, a delete-vs-edit, or an add/add with different text). The
    /// keys are the offending items, sorted. The caller leaves git's
    /// conflict markers — we only ever *reduce* the conflict count.
    Conflict(Vec<String>),
    /// We could not produce a result we trust. The caller also leaves
    /// git's markers. See [`BailReason`].
    Bail(BailReason),
}

/// Why a structural merge declined to produce a resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BailReason {
    /// One of base/ours/theirs failed to parse (error/missing node) — we
    /// never operate on an untrusted tree.
    SideParseFailed,
    /// A side had two top-level items with the same identity key, so the
    /// per-key 3-way classify would be ambiguous. Conservatively refuse.
    DuplicateKeys,
    /// The reconstructed source did not parse cleanly — discard it rather
    /// than emit a tree we can't trust.
    ReparseFailed,
}

/// Structural 3-way merge of the top-level items of `base`/`ours`/`theirs`
/// (all the same `lang`). Pure. See [`AstMerge`] for the outcomes.
///
/// Resolution rule (per the spike, lifted to production): per identity
/// key, take the side that changed, take agreement, and refuse the whole
/// file on any divergent same-key edit, delete-vs-edit, or add/add with
/// different text. Ordering is conservative and deterministic: base order
/// for surviving items, with each side's additions inserted after their
/// nearest surviving base anchor (ours before theirs on ties). The
/// reconstruction reuses each item's verbatim byte-span text (no reflow)
/// and is re-parsed as a guard before being returned.
pub fn merge_top_level(base: &str, ours: &str, theirs: &str, lang: &Language) -> AstMerge {
    merge_with_reconstruct(base, ours, theirs, lang, reconstruct)
}

/// Inner pipeline parameterized on the reconstruction function so the
/// re-parse guard's discard path is directly testable (inject a
/// reconstructor that emits broken source and assert `Bail(ReparseFailed)`).
fn merge_with_reconstruct(
    base: &str,
    ours: &str,
    theirs: &str,
    lang: &Language,
    reconstruct_fn: impl Fn(&[Item]) -> String,
) -> AstMerge {
    let (Some(b_items), Some(o_items), Some(t_items)) = (
        parse_top_level_items(base, lang),
        parse_top_level_items(ours, lang),
        parse_top_level_items(theirs, lang),
    ) else {
        return AstMerge::Bail(BailReason::SideParseFailed);
    };

    let (Some(bmap), Some(omap), Some(tmap)) = (
        index_by_key(&b_items),
        index_by_key(&o_items),
        index_by_key(&t_items),
    ) else {
        return AstMerge::Bail(BailReason::DuplicateKeys);
    };

    // Classify every key once. `resolved` holds the winning item per
    // surviving key; dropped keys are simply absent.
    let mut all_keys: Vec<&str> = bmap
        .keys()
        .chain(omap.keys())
        .chain(tmap.keys())
        .copied()
        .collect();
    all_keys.sort_unstable();
    all_keys.dedup();

    let mut resolved: BTreeMap<&str, &Item> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    for &key in &all_keys {
        match classify(
            bmap.get(key).copied(),
            omap.get(key).copied(),
            tmap.get(key).copied(),
        ) {
            Ok(Some(item)) => {
                resolved.insert(key, item);
            }
            Ok(None) => {}
            Err(()) => conflicts.push(key.to_string()),
        }
    }
    if !conflicts.is_empty() {
        return AstMerge::Conflict(conflicts);
    }

    let merged = order_items(&b_items, &o_items, &t_items, &bmap, &omap, &resolved);
    let text = reconstruct_fn(&merged);

    // Re-parse guard: never hand back a tree we can't re-parse.
    if parse_top_level_items(&text, lang).is_none() {
        return AstMerge::Bail(BailReason::ReparseFailed);
    }
    AstMerge::Resolved(text)
}

/// Index items by identity key. `None` if a key repeats within one side
/// (ambiguous — the caller bails).
fn index_by_key(items: &[Item]) -> Option<BTreeMap<&str, &Item>> {
    let mut map = BTreeMap::new();
    for it in items {
        if map.insert(it.key.as_str(), it).is_some() {
            return None;
        }
    }
    Some(map)
}

/// Per-key 3-way classify. `Ok(Some)` ⇒ keep that item; `Ok(None)` ⇒
/// drop (deleted); `Err(())` ⇒ true overlap (refuse the file).
fn classify<'a>(
    b: Option<&'a Item>,
    o: Option<&'a Item>,
    t: Option<&'a Item>,
) -> Result<Option<&'a Item>, ()> {
    match (b, o, t) {
        // Present everywhere: classic 3-way on the verbatim item text.
        (Some(b), Some(o), Some(t)) => {
            if o.text == t.text {
                Ok(Some(o)) // both agree (incl. both-made-same-edit)
            } else if o.text == b.text {
                Ok(Some(t)) // only theirs changed
            } else if t.text == b.text {
                Ok(Some(o)) // only ours changed
            } else {
                Err(()) // divergent same-key edit
            }
        }
        // Deleted by one side, untouched by the other → drop.
        (Some(b), None, Some(t)) if t.text == b.text => Ok(None),
        (Some(b), Some(o), None) if o.text == b.text => Ok(None),
        // Delete vs edit — true overlap.
        (Some(_), None, Some(_)) | (Some(_), Some(_), None) => Err(()),
        // Deleted by both → drop.
        (Some(_), None, None) => Ok(None),
        // Added by exactly one side → the commutative win.
        (None, Some(o), None) => Ok(Some(o)),
        (None, None, Some(t)) => Ok(Some(t)),
        // Added by both: fine iff identical, else add/add overlap.
        (None, Some(o), Some(t)) => {
            if o.text == t.text {
                Ok(Some(o))
            } else {
                Err(())
            }
        }
        (None, None, None) => unreachable!("key came from some side"),
    }
}

/// Build the merged item list in a deterministic, conservative order:
/// base order for surviving base items, with each side's new items
/// inserted after their nearest surviving base anchor (ours before
/// theirs; items before any base anchor go to the front, ours first).
fn order_items<'a>(
    b_items: &'a [Item],
    o_items: &'a [Item],
    t_items: &'a [Item],
    bmap: &BTreeMap<&str, &Item>,
    omap: &BTreeMap<&str, &Item>,
    resolved: &BTreeMap<&str, &'a Item>,
) -> Vec<Item> {
    let base_keys: HashSet<&str> = bmap.keys().copied().collect();

    // Front bucket (no surviving base anchor precedes them) and
    // anchor → items inserted directly after that base key.
    let mut front: Vec<&Item> = Vec::new();
    let mut after: HashMap<&str, Vec<&Item>> = HashMap::new();

    let mut place = |side: &'a [Item], skip_both_add: bool| {
        let mut anchor: Option<&str> = None;
        for it in side {
            let k = it.key.as_str();
            if base_keys.contains(k) {
                // Only a *surviving* base item is a valid anchor.
                if resolved.contains_key(k) {
                    anchor = Some(k);
                }
                continue;
            }
            // An addition from this side. Skip both-add keys on the
            // theirs pass — ours already placed them.
            if skip_both_add && omap.contains_key(k) {
                continue;
            }
            if let Some(&item) = resolved.get(k) {
                match anchor {
                    Some(a) => after.entry(a).or_default().push(item),
                    None => front.push(item),
                }
            }
        }
    };
    place(o_items, false);
    place(t_items, true);

    let mut ordered: Vec<&Item> = Vec::new();
    ordered.extend(front);
    for it in b_items {
        let k = it.key.as_str();
        if let Some(&winner) = resolved.get(k) {
            ordered.push(winner);
            if let Some(extra) = after.get(k) {
                ordered.extend(extra.iter().copied());
            }
        }
    }

    ordered.into_iter().cloned().collect()
}

/// Reconstruct file text from ordered items. Each item's verbatim span
/// text is reused unchanged (no reflow); only the inter-item joins (a
/// single newline) are synthesized. Trailing newline included.
fn reconstruct(items: &[Item]) -> String {
    let mut out = String::new();
    for it in items {
        out.push_str(it.text.trim_end());
        out.push('\n');
        // A standalone comment block was detached by a blank line — that blank
        // line is what made it standalone, so put it back rather than gluing a
        // licence header onto the first declaration.
        if it.key.starts_with(COMMENT_KEY_PREFIX) {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_code_metrics::for_tests::{lang, shipped};

    /// Assert `src[item.byte_span] == item.text` for every item — the
    /// span is the authoritative source range.
    fn assert_spans_match(src: &str, items: &[Item]) {
        for it in items {
            assert_eq!(
                &src[it.byte_span.clone()],
                it.text,
                "span/text mismatch for {}",
                it.key
            );
        }
    }

    fn keys(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.key.as_str()).collect()
    }

    /// a comment run separated from the next declaration by a blank
    /// line — a licence header, a section divider, a trailing note — belongs to
    /// no item, so `reconstruct` (which emits only `Item.text`) dropped it.
    /// The re-parse guard cannot catch this: source with the header removed
    /// parses perfectly.
    ///
    /// This falsified the documented invariant that Tier-2 "can only ever
    /// reduce the conflict count, never introduce new content" — it *removed*
    /// content, silently, in the most common file shape there is, and reported
    /// success.
    #[test]
    fn detached_comment_runs_survive_a_structural_merge() {
        let base = "// Copyright 2026 ACME Corp.\n\
                    // SPDX-License-Identifier: MIT\n\
                    \n\
                    use std::fmt;\n\
                    \n\
                    pub fn run() -> u32 {\n    1\n}\n\
                    \n\
                    // TODO: revisit the retry policy\n";
        let ours = base.replace("use std::fmt;", "use std::fmt;\nuse std::io;");
        let theirs = base.replace("use std::fmt;", "use std::fmt;\nuse std::net;");

        let AstMerge::Resolved(merged) = merge_top_level(base, &ours, &theirs, &lang("rust"))
        else {
            panic!("both sides adding a different import must resolve structurally");
        };

        assert!(
            merged.contains("// Copyright 2026 ACME Corp."),
            "licence header dropped:\n{merged}"
        );
        assert!(
            merged.contains("// SPDX-License-Identifier: MIT"),
            "SPDX line dropped:\n{merged}"
        );
        assert!(
            merged.contains("// TODO: revisit the retry policy"),
            "trailing comment at EOF dropped:\n{merged}"
        );
        // The merge itself still has to work.
        assert!(merged.contains("use std::io;"), "{merged}");
        assert!(merged.contains("use std::net;"), "{merged}");
        assert!(merged.contains("pub fn run()"), "{merged}");
        // And the header must stay at the top, not float into the body.
        let header = merged.find("// Copyright").expect("header present");
        let first_decl = merged.find("use std::").expect("an import present");
        assert!(
            header < first_decl,
            "header must precede the declarations:\n{merged}"
        );
    }

    #[test]
    fn a_detached_comment_run_is_parsed_as_its_own_item() {
        let src = "// header\n\nuse std::fmt;\n\n// trailing\n";
        let items = parse_top_level_items(src, &lang("rust")).expect("parses");
        assert_spans_match(src, &items);
        // header, the use, trailing — three items, in source order.
        assert_eq!(items.len(), 3, "got keys {:?}", keys(&items));
        assert!(items[0].text.contains("// header"));
        assert!(items[1].text.contains("use std::fmt;"));
        assert!(items[2].text.contains("// trailing"));
    }

    #[test]
    fn detached_comments_survive_in_python_too() {
        // The fix rides tree-sitter's `is_extra()`, so it should be
        // language-agnostic — but the original report named Python explicitly,
        // so pin it rather than assume.
        let base = "# Copyright 2026 ACME Corp.\n\nimport os\n\ndef run():\n    return 1\n";
        let ours = base.replace("import os", "import os\nimport sys");
        let theirs = base.replace("import os", "import os\nimport json");
        let AstMerge::Resolved(merged) = merge_top_level(base, &ours, &theirs, &lang("python"))
        else {
            panic!("independent imports must resolve structurally");
        };
        assert!(
            merged.contains("# Copyright 2026 ACME Corp."),
            "header dropped:\n{merged}"
        );
        assert!(merged.contains("import sys"), "{merged}");
        assert!(merged.contains("import json"), "{merged}");
    }

    #[test]
    fn an_attached_doc_comment_still_rides_with_its_declaration() {
        // The existing behaviour must not regress: no blank line means the
        // comment belongs to the decl and moves with it.
        let src = "/// docs for run\npub fn run() {}\n";
        let items = parse_top_level_items(src, &lang("rust")).expect("parses");
        assert_eq!(items.len(), 1, "got keys {:?}", keys(&items));
        assert!(items[0].text.contains("/// docs for run"));
        assert!(items[0].text.contains("pub fn run()"));
    }

    #[test]
    fn language_for_path_maps_supported_extensions() {
        assert_eq!(
            language_for_path(&shipped(), "a/b.rs").map(|l| l.name()),
            Some("rust")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.ts").map(|l| l.name()),
            Some("typescript")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.tsx").map(|l| l.name()),
            Some("tsx")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.js").map(|l| l.name()),
            Some("javascript")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.mjs").map(|l| l.name()),
            Some("javascript")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.jsx").map(|l| l.name()),
            Some("javascript")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.py").map(|l| l.name()),
            Some("python")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.go").map(|l| l.name()),
            Some("go")
        );
        // Second-slice languages.
        assert_eq!(
            language_for_path(&shipped(), "x.java").map(|l| l.name()),
            Some("java")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.c").map(|l| l.name()),
            Some("c")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.h").map(|l| l.name()),
            Some("c")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.cpp").map(|l| l.name()),
            Some("cpp")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.hpp").map(|l| l.name()),
            Some("cpp")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.clj").map(|l| l.name()),
            Some("clojure")
        );
        assert_eq!(
            language_for_path(&shipped(), "x.cljs").map(|l| l.name()),
            Some("clojure")
        );
        // Genuinely unsupported.
        assert_eq!(
            language_for_path(&shipped(), "x.txt").map(|l| l.name()),
            None
        );
        assert_eq!(
            language_for_path(&shipped(), "noext").map(|l| l.name()),
            None
        );
    }

    #[test]
    fn csharp_is_canonical_but_unsupported_for_merge() {
        // C# lives in the unified `Language` enum — it's analysed
        // for metrics — but the AST merge front half doesn't support it, so
        // it has no `MergeSpec`, `parse_top_level_items` bails gracefully,
        // and a `.cs` path is never picked up by the merge resolver.
        assert!(merge_spec(&lang("csharp")).is_none());
        assert!(merge_spec(&lang("rust")).is_some());
        assert!(parse_top_level_items("class C { void M() {} }", &lang("csharp")).is_none());
        assert_eq!(
            language_for_path(&shipped(), "x.cs").map(|l| l.name()),
            None
        );
    }

    #[test]
    fn rust_extracts_top_level_items_with_keys() {
        let src = "use std::fmt;\nextern crate foo;\nfn a() {}\nstruct S;\nenum E { A }\n\
                   const K: u8 = 1;\ntrait T {}\nimpl S {}\n";
        let items = parse_top_level_items(src, &lang("rust")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::use std::fmt;"));
        assert!(k.contains(&"import::extern crate foo;"));
        assert!(k.contains(&"function_item::a"));
        assert!(k.contains(&"struct_item::S"));
        assert!(k.contains(&"enum_item::E"));
        assert!(k.contains(&"const_item::K"));
        assert!(k.contains(&"trait_item::T"));
        // Bare impl has no name field → kind + normalized text fallback.
        assert!(k.iter().any(|s| s.starts_with("impl_item::")));
    }

    #[test]
    fn rust_attaches_leading_doc_comment_to_following_item() {
        let src = "/// doc for a\nfn a() {}\n\n// detached\n\nfn b() {}\n";
        let items = parse_top_level_items(src, &lang("rust")).unwrap();
        assert_spans_match(src, &items);
        let a = items.iter().find(|i| i.key == "function_item::a").unwrap();
        assert!(
            a.text.starts_with("/// doc for a"),
            "doc comment should be in the span: {:?}",
            a.text
        );
        // The blank-line-separated comment does NOT glue onto `b`.
        let b = items.iter().find(|i| i.key == "function_item::b").unwrap();
        assert!(
            b.text.starts_with("fn b"),
            "detached comment must not attach: {:?}",
            b.text
        );
    }

    #[test]
    fn rust_bails_on_parse_failure() {
        // Unbalanced brace → tree-sitter produces an ERROR node.
        assert!(parse_top_level_items("fn a() { ", &lang("rust")).is_none());
    }

    #[test]
    fn typescript_imports_classes_and_exported_decls() {
        let src = "import { x } from 'm';\nfunction f() {}\nclass C {}\n\
                   interface I {}\ntype T = number;\nenum E { A }\n\
                   export function g() {}\nexport class D {}\nconst k = 1;\n";
        let items = parse_top_level_items(src, &lang("typescript")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import { x } from 'm';"));
        assert!(k.contains(&"function_declaration::f"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"interface_declaration::I"));
        assert!(k.contains(&"type_alias_declaration::T"));
        assert!(k.contains(&"enum_declaration::E"));
        // export wrapper is seen through to the inner decl identity.
        assert!(k.contains(&"function_declaration::g"));
        assert!(k.contains(&"class_declaration::D"));
        // `const` lands on the text fallback (declarator-named).
        assert!(k.iter().any(|s| s.starts_with("lexical_declaration::")));
    }

    #[test]
    fn tsx_parses_component_and_keeps_export_identity() {
        let src = "import React from 'react';\n\
                   export function App() { return <div>hi</div>; }\n";
        let items = parse_top_level_items(src, &lang("tsx")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import React from 'react';"));
        assert!(k.contains(&"function_declaration::App"));
    }

    #[test]
    fn javascript_imports_and_functions() {
        let src = "import x from 'm';\nfunction f() {}\nclass C {}\nexport function g() {}\n";
        let items = parse_top_level_items(src, &lang("javascript")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import x from 'm';"));
        assert!(k.contains(&"function_declaration::f"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"function_declaration::g"));
    }

    #[test]
    fn python_imports_defs_classes_and_decorated() {
        let src = "import os\nfrom sys import path\n\
                   def f():\n    pass\nclass C:\n    pass\n\
                   @deco\ndef g():\n    pass\n";
        let items = parse_top_level_items(src, &lang("python")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import os"));
        assert!(k.contains(&"import::from sys import path"));
        assert!(k.contains(&"function_definition::f"));
        assert!(k.contains(&"class_definition::C"));
        // decorated def: keyed by the wrapped function, decorator in text.
        assert!(k.contains(&"function_definition::g"));
        let g = items
            .iter()
            .find(|i| i.key == "function_definition::g")
            .unwrap();
        assert!(
            g.text.starts_with("@deco"),
            "decorator rides along: {:?}",
            g.text
        );
    }

    #[test]
    fn python_bails_on_parse_failure() {
        // A `def` with no body is a syntax error in Python.
        assert!(parse_top_level_items("def f(:\n", &lang("python")).is_none());
    }

    #[test]
    fn go_package_imports_funcs_methods_and_types() {
        let src = "package main\n\
                   import \"fmt\"\n\
                   func F() {}\n\
                   func (s S) M() {}\n\
                   type S struct{}\n\
                   const K = 1\n";
        let items = parse_top_level_items(src, &lang("go")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import \"fmt\""));
        assert!(k.contains(&"function_declaration::F"));
        // Method disambiguated by receiver.
        assert!(
            k.iter()
                .any(|s| s.starts_with("method_declaration::") && s.ends_with("::M")),
            "method keyed with receiver: {k:?}"
        );
        // package clause and grouped type/const land on text fallback.
        assert!(k.iter().any(|s| s.starts_with("package_clause::")));
        assert!(k.iter().any(|s| s.starts_with("type_declaration::")));
    }

    #[test]
    fn java_imports_and_type_declarations() {
        let src = "package p;\nimport java.util.List;\n\
                   class C { int x; void m(int a) {} }\n\
                   interface I {}\nenum E { A }\nrecord R(int a) {}\n";
        let items = parse_top_level_items(src, &lang("java")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import java.util.List;"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"interface_declaration::I"));
        assert!(k.contains(&"enum_declaration::E"));
        assert!(k.contains(&"record_declaration::R"));
        // package clause has no name field → text fallback.
        assert!(k.iter().any(|s| s.starts_with("package_declaration::")));
    }

    #[test]
    fn c_includes_functions_structs_typedefs_globals() {
        let src = "#include <stdio.h>\nint g = 1;\ntypedef int myint;\n\
                   struct S { int a; };\nint main(void) { return 0; }\n\
                   void foo(int x) {}\n";
        let items = parse_top_level_items(src, &lang("c")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::#include <stdio.h>"));
        assert!(k.contains(&"declaration::g"));
        assert!(k.contains(&"type_definition::myint"));
        assert!(k.contains(&"struct_specifier::S"));
        // Functions keyed by declarator (name + signature).
        assert!(k.contains(&"function_definition::main(void)"));
        assert!(k.contains(&"function_definition::foo(int x)"));
    }

    #[test]
    fn cpp_overloads_stay_distinct_and_template_seen_through() {
        let src = "#include <vector>\nnamespace ns { int v; }\n\
                   class C {};\nstruct S {};\n\
                   template<typename T> T id(T x) { return x; }\n\
                   void f(int a) {}\nvoid f(double a) {}\n";
        let items = parse_top_level_items(src, &lang("cpp")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::#include <vector>"));
        assert!(k.contains(&"namespace_definition::ns"));
        assert!(k.contains(&"class_specifier::C"));
        assert!(k.contains(&"struct_specifier::S"));
        // Overloads keyed by signature stay distinct.
        assert!(k.contains(&"function_definition::f(int a)"));
        assert!(k.contains(&"function_definition::f(double a)"));
        // template_declaration is seen through to the wrapped function.
        assert!(k.contains(&"function_definition::id(T x)"));
    }

    #[test]
    fn clojure_def_forms_keyed_by_head_and_name() {
        let src = "(ns my.app)\n(def x 1)\n(defn foo [a] a)\n\
                   (defn- bar [] 2)\n(defmacro m [] nil)\n";
        let items = parse_top_level_items(src, &lang("clojure")).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"ns::my.app"));
        assert!(k.contains(&"def::x"));
        assert!(k.contains(&"defn::foo"));
        assert!(k.contains(&"defn-::bar"));
        assert!(k.contains(&"defmacro::m"));
    }

    #[test]
    fn java_both_add_classes_resolve() {
        let base = "import a.A;\nclass Base {}\n";
        let ours = "import a.A;\nclass Base {}\nclass Ours {}\n";
        let theirs = "import a.A;\nclass Base {}\nclass Theirs {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("java")));
        assert!(text.contains("class Ours"), "{text}");
        assert!(text.contains("class Theirs"), "{text}");
        assert!(parse_top_level_items(&text, &lang("java")).is_some());
    }

    #[test]
    fn c_both_add_functions_resolve() {
        let base = "int shared(void) { return 0; }\n";
        let ours = "int shared(void) { return 0; }\nint ours(void) { return 1; }\n";
        let theirs = "int shared(void) { return 0; }\nint theirs(void) { return 2; }\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("c")));
        assert!(text.contains("int ours"), "{text}");
        assert!(text.contains("int theirs"), "{text}");
        assert!(parse_top_level_items(&text, &lang("c")).is_some());
    }

    #[test]
    fn c_same_function_edited_differently_refuses() {
        let base = "int f(void) { return 1; }\n";
        let ours = "int f(void) { return 2; }\n";
        let theirs = "int f(void) { return 3; }\n";
        match merge_top_level(base, ours, theirs, &lang("c")) {
            AstMerge::Conflict(keys) => assert!(
                keys.iter()
                    .any(|k| k.starts_with("function_definition::f(void)")),
                "{keys:?}"
            ),
            other => panic!("must refuse divergent body edit, got {other:?}"),
        }
    }

    #[test]
    fn cpp_both_add_imports_resolve() {
        let base = "#include <a>\nint main() { return 0; }\n";
        let ours = "#include <a>\n#include <b>\nint main() { return 0; }\n";
        let theirs = "#include <a>\n#include <c>\nint main() { return 0; }\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("cpp")));
        assert!(text.contains("#include <b>"), "{text}");
        assert!(text.contains("#include <c>"), "{text}");
        assert!(parse_top_level_items(&text, &lang("cpp")).is_some());
    }

    #[test]
    fn clojure_both_add_defns_resolve() {
        let base = "(ns app)\n(defn shared [] 0)\n";
        let ours = "(ns app)\n(defn shared [] 0)\n(defn ours [] 1)\n";
        let theirs = "(ns app)\n(defn shared [] 0)\n(defn theirs [] 2)\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("clojure")));
        assert!(text.contains("(defn ours"), "{text}");
        assert!(text.contains("(defn theirs"), "{text}");
        assert!(parse_top_level_items(&text, &lang("clojure")).is_some());
    }

    #[test]
    fn clojure_same_defn_edited_differently_refuses() {
        let base = "(defn f [] 1)\n";
        let ours = "(defn f [] 2)\n";
        let theirs = "(defn f [] 3)\n";
        match merge_top_level(base, ours, theirs, &lang("clojure")) {
            AstMerge::Conflict(keys) => {
                assert!(keys.iter().any(|k| k == "defn::f"), "{keys:?}")
            }
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn empty_source_yields_no_items() {
        assert!(parse_top_level_items("", &lang("rust")).unwrap().is_empty());
        assert!(parse_top_level_items("\n\n", &lang("python"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ordering_is_source_order() {
        let src = "fn first() {}\nfn second() {}\nfn third() {}\n";
        let items = parse_top_level_items(src, &lang("rust")).unwrap();
        let k = keys(&items);
        assert_eq!(
            k,
            vec![
                "function_item::first",
                "function_item::second",
                "function_item::third"
            ]
        );
    }

    // ---- per-item 3-way merge ----

    fn resolved_text(m: AstMerge) -> String {
        match m {
            AstMerge::Resolved(s) => s,
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn rust_both_add_different_imports_resolves_in_order() {
        // The flagship Tier-2 case: two new `use`s, order-insensitive.
        let base = "use std::fmt;\n";
        let ours = "use std::collections::HashMap;\nuse std::fmt;\n";
        let theirs = "use std::fmt;\nuse std::io;\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("rust")));
        // ours-add (front) → base survivor → theirs-add (anchored after).
        assert_eq!(
            text,
            "use std::collections::HashMap;\nuse std::fmt;\nuse std::io;\n"
        );
    }

    #[test]
    fn rust_independently_added_fns_resolve() {
        let base = "fn shared() {}\n";
        let ours = "fn shared() {}\nfn from_ours() {}\n";
        let theirs = "fn shared() {}\nfn from_theirs() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("rust")));
        assert!(text.contains("fn from_ours"), "{text}");
        assert!(text.contains("fn from_theirs"), "{text}");
        // Both new fns anchor after the surviving `shared`, ours first.
        let o = text.find("from_ours").unwrap();
        let t = text.find("from_theirs").unwrap();
        let s = text.find("fn shared").unwrap();
        assert!(
            s < o && o < t,
            "order ours-before-theirs after anchor: {text}"
        );
    }

    #[test]
    fn rust_body_edit_on_one_side_add_on_other_resolves() {
        let base = "fn a() { 1 }\n";
        let ours = "fn a() { 2 }\n"; // body edit
        let theirs = "fn a() { 1 }\nfn b() {}\n"; // new fn
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("rust")));
        assert!(text.contains("fn a() { 2 }"), "edited body wins: {text}");
        assert!(text.contains("fn b()"), "{text}");
    }

    #[test]
    fn rust_both_delete_same_item_resolves() {
        let base = "fn a() {}\nfn b() {}\n";
        let ours = "fn b() {}\n";
        let theirs = "fn b() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("rust")));
        assert!(!text.contains("fn a()"), "a deleted by both: {text}");
        assert!(text.contains("fn b()"), "{text}");
    }

    #[test]
    fn rust_same_item_edited_differently_refuses() {
        let base = "fn calc() { a() }\n";
        let ours = "fn calc() { b() }\n";
        let theirs = "fn calc() { c() }\n";
        match merge_top_level(base, ours, theirs, &lang("rust")) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::calc")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse divergent same-item edit, got {other:?}"),
        }
    }

    #[test]
    fn rust_add_add_with_different_bodies_refuses() {
        let base = "fn keep() {}\n";
        let ours = "fn keep() {}\nfn n() { 1 }\n";
        let theirs = "fn keep() {}\nfn n() { 2 }\n";
        match merge_top_level(base, ours, theirs, &lang("rust")) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::n")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse add/add divergence, got {other:?}"),
        }
    }

    #[test]
    fn rust_delete_vs_edit_refuses() {
        let base = "fn a() { 1 }\n";
        let ours = "\n"; // a deleted
        let theirs = "fn a() { 2 }\n"; // a edited
        match merge_top_level(base, ours, theirs, &lang("rust")) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::a")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse delete-vs-edit, got {other:?}"),
        }
    }

    #[test]
    fn ordering_front_additions_are_deterministic_ours_then_theirs() {
        // Both sides add a fn *before* the surviving base item → both land
        // in the front bucket, ours first, deterministically.
        let base = "fn base() {}\n";
        let ours = "fn o_new() {}\nfn base() {}\n";
        let theirs = "fn t_new() {}\nfn base() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("rust")));
        assert_eq!(text, "fn o_new() {}\nfn t_new() {}\nfn base() {}\n");
    }

    #[test]
    fn reparse_guard_discards_broken_reconstruction() {
        // Inject a reconstructor that emits un-parseable source; even
        // though the classify is clean, the guard must bail.
        let base = "use std::fmt;\n";
        let ours = "use std::io;\nuse std::fmt;\n";
        let theirs = "use std::fmt;\nuse std::cmp;\n";
        let out = merge_with_reconstruct(base, ours, theirs, &lang("rust"), |_| {
            "fn broken( {\n".to_string()
        });
        assert_eq!(out, AstMerge::Bail(BailReason::ReparseFailed));
        // Sanity: the real reconstructor resolves the same inputs.
        assert!(matches!(
            merge_top_level(base, ours, theirs, &lang("rust")),
            AstMerge::Resolved(_)
        ));
    }

    #[test]
    fn unparseable_side_bails() {
        let base = "fn a() {}\n";
        let ours = "fn a() { "; // unbalanced → parse error
        let theirs = "fn a() {}\nfn b() {}\n";
        assert_eq!(
            merge_top_level(base, ours, theirs, &lang("rust")),
            AstMerge::Bail(BailReason::SideParseFailed)
        );
    }

    #[test]
    fn typescript_both_add_imports_resolve() {
        let base = "import a from 'a';\n";
        let ours = "import b from 'b';\nimport a from 'a';\n";
        let theirs = "import a from 'a';\nimport c from 'c';\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("typescript")));
        assert_eq!(
            text,
            "import b from 'b';\nimport a from 'a';\nimport c from 'c';\n"
        );
    }

    #[test]
    fn javascript_body_edit_vs_add_resolves() {
        let base = "function a() { return 1; }\n";
        let ours = "function a() { return 2; }\n";
        let theirs = "function a() { return 1; }\nfunction b() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("javascript")));
        assert!(text.contains("return 2"), "edited body wins: {text}");
        assert!(text.contains("function b()"), "{text}");
    }

    #[test]
    fn python_both_add_functions_resolve() {
        let base = "def shared():\n    pass\n";
        let ours = "def shared():\n    pass\ndef o():\n    pass\n";
        let theirs = "def shared():\n    pass\ndef t():\n    pass\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("python")));
        assert!(text.contains("def o():"), "{text}");
        assert!(text.contains("def t():"), "{text}");
        // Reconstruction must itself be valid Python (re-parse guard passed).
        assert!(parse_top_level_items(&text, &lang("python")).is_some());
    }

    #[test]
    fn python_same_def_edited_differently_refuses() {
        let base = "def f():\n    return 1\n";
        let ours = "def f():\n    return 2\n";
        let theirs = "def f():\n    return 3\n";
        match merge_top_level(base, ours, theirs, &lang("python")) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_definition::f")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn go_both_add_functions_resolve_keeping_package_first() {
        let base = "package main\nfunc S() {}\n";
        let ours = "package main\nfunc S() {}\nfunc O() {}\n";
        let theirs = "package main\nfunc S() {}\nfunc T() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, &lang("go")));
        assert!(
            text.starts_with("package main"),
            "package stays first: {text}"
        );
        assert!(text.contains("func O()"), "{text}");
        assert!(text.contains("func T()"), "{text}");
        assert!(parse_top_level_items(&text, &lang("go")).is_some());
    }
}
