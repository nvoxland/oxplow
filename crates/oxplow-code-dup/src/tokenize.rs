//! Compute Deckard-style subtree hashes from a tree-sitter AST.
//!
//! A subtree's hash is a fold over its preorder kind sequence with
//! identifiers/literals normalized (so renames don't break the
//! match) and import / use / include / package declarations skipped
//! whole-subtree (so two files that share a few imports don't seed
//! a fingerprint collision). The hash is salted by `Language::tag()` (a
//! hash of its id)
//! so cross-language structures cannot accidentally collide.
//!
//! The same walker is used by every caller. The unit of duplication
//! detection is "the hash of a subtree" — not a sliding token
//! window.

use oxplow_code_metrics::{form_head, Language};
use tree_sitter::Node;

/// Result of hashing one subtree.
#[derive(Debug, Clone, Copy)]
pub struct SubtreeHash {
    /// 64-bit fold of the preorder normalized kind sequence.
    pub hash: u64,
    /// Number of AST nodes in the subtree (after skip-pruning, after
    /// comment-pruning). Cheap proxy for "how interesting is this
    /// subtree" — small subtrees are filtered before reporting.
    pub node_count: u32,
    pub start_line: u32,
    pub end_line: u32,
}

/// Hash one subtree.
pub fn hash_subtree(root: Node<'_>, src: &[u8], lang: &Language) -> SubtreeHash {
    let mut acc: u64 = 0xcbf29ce484222325 ^ lang.tag();
    let mut count: u32 = 0;
    let mut cursor = root.walk();
    walk_hash(&mut cursor, src, lang, &mut acc, &mut count);
    SubtreeHash {
        hash: acc,
        node_count: count,
        start_line: root.start_position().row as u32 + 1,
        end_line: root.end_position().row as u32 + 1,
    }
}

fn walk_hash(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    src: &[u8],
    lang: &Language,
    acc: &mut u64,
    count: &mut u32,
) {
    let node = cursor.node();
    let kind = node.kind();
    if is_comment_kind(kind) {
        return;
    }
    if is_skip_node(lang, node, src) {
        return;
    }
    let normalized = normalize_kind(kind);
    fold_str(acc, normalized);
    *count += 1;
    if cursor.goto_first_child() {
        loop {
            walk_hash(cursor, src, lang, acc, count);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
    // Mark the closing of a subtree by folding a sentinel — without
    // this, "if { x }" and "{ if x }" can hash the same.
    fold_byte(acc, 0xFF);
}

#[inline]
fn fold_str(acc: &mut u64, s: &str) {
    for &b in s.as_bytes() {
        *acc ^= b as u64;
        *acc = acc.wrapping_mul(0x00000100000001B3);
    }
    fold_byte(acc, 0xFE); // separator between sibling kind tokens
}

#[inline]
fn fold_byte(acc: &mut u64, b: u8) {
    *acc ^= b as u64;
    *acc = acc.wrapping_mul(0x00000100000001B3);
}

/// Exact-match comment node kinds across our 9 grammars.
pub fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "doc_comment"
    )
}

/// Nodes whose entire subtree is skipped: imports, use, include and
/// package directives are noise (they share structure across unrelated
/// files). Each language lists them (`duplication.skip_kinds`); a grammar of
/// generic forms names the forms' heads (`duplication.skip_form_heads`).
pub fn is_skip_node(lang: &Language, node: Node<'_>, src: &[u8]) -> bool {
    let spec = lang.spec();
    let dup = &spec.duplication;
    if dup.skip_kinds.iter().any(|k| k == node.kind()) {
        return true;
    }
    !dup.skip_form_heads.is_empty()
        && form_head(node, src, spec).is_some_and(|h| dup.skip_form_heads.iter().any(|s| s == h))
}

/// Coarse token-kind buckets — fold identifiers and literals into
/// generic placeholders so renames + literal swaps don't break
/// matches, but keep structural punctuation distinct.
pub fn normalize_kind(kind: &str) -> &str {
    if matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "primitive_type"
            | "scoped_identifier"
            | "scoped_type_identifier"
    ) {
        return "ID";
    }
    if matches!(
        kind,
        "integer_literal"
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "float_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "number"
            | "integer"
            | "float"
    ) {
        return "NUM";
    }
    if matches!(
        kind,
        "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "string"
            | "string_fragment"
            | "interpreted_string_literal"
            | "raw_string_fragment"
    ) {
        return "STR";
    }
    kind
}
