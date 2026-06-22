//! Compute Deckard-style subtree hashes from a tree-sitter AST.
//!
//! A subtree's hash is a fold over its preorder kind sequence with
//! identifiers/literals normalized (so renames don't break the
//! match) and import / use / include / package declarations skipped
//! whole-subtree (so two files that share a few imports don't seed
//! a fingerprint collision). The hash is salted by `Language::tag()`
//! so cross-language structures cannot accidentally collide.
//!
//! The same walker is used by every caller. The unit of duplication
//! detection is "the hash of a subtree" — not a sliding token
//! window.

use oxplow_code_metrics::{head_symbol_text, Language};
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
pub fn hash_subtree(root: Node<'_>, src: &[u8], lang: Language) -> SubtreeHash {
    let mut acc: u64 = 0xcbf29ce484222325 ^ (lang.tag() as u64);
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
    lang: Language,
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

/// Per-language predicate for nodes whose entire subtree we skip.
/// Imports / use / include / package directives are noise: they
/// share lots of structure across unrelated files. Skipping
/// whole-subtree means whatever the grammar wraps inside (paths,
/// aliases, brace-lists) contributes nothing to the hash. For
/// most languages this is a static `node.kind()` check; Clojure
/// inspects the head symbol of a `list_lit` because every form
/// shares one node kind.
pub fn is_skip_node(lang: Language, node: Node<'_>, src: &[u8]) -> bool {
    let kind = node.kind();
    match lang {
        Language::Rust => matches!(kind, "use_declaration" | "extern_crate_declaration"),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            matches!(kind, "import_statement" | "import_alias")
        }
        Language::Python => matches!(
            kind,
            "import_statement" | "import_from_statement" | "future_import_statement"
        ),
        Language::Go => matches!(
            kind,
            "import_declaration" | "import_spec" | "import_spec_list" | "package_clause"
        ),
        Language::Java => matches!(kind, "import_declaration" | "package_declaration"),
        Language::C => matches!(kind, "preproc_include"),
        Language::Cpp => matches!(
            kind,
            "preproc_include" | "using_declaration" | "using_directive"
        ),
        Language::CSharp => matches!(kind, "using_directive"),
        Language::Clojure => {
            if kind != "list_lit" {
                return false;
            }
            match head_symbol_text(node, src) {
                Some(head) => matches!(head, "ns" | "require" | "import" | "use"),
                None => false,
            }
        }
    }
}

/// Coarse token-kind buckets — fold identifiers and literals into
/// generic placeholders so renames + literal swaps don't break
/// matches, but keep structural punctuation distinct.
pub fn normalize_kind(kind: &str) -> &'static str {
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
    static_str(kind)
}

#[inline]
fn static_str(s: &str) -> &'static str {
    // SAFETY: `Node::kind()` always returns a `&'static str` produced
    // by the tree-sitter C grammar tables. The `&str` we receive here
    // is one of those static strings; only the lifetime annotation is
    // narrower. Re-extending it back to `'static` is sound.
    unsafe { std::mem::transmute::<&str, &'static str>(s) }
}
