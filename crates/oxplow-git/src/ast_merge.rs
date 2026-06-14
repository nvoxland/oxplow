//! Tier-2 AST structural merge — parse → top-level-items front half (tsk134).
//!
//! This is the language-neutral *foundation* of the AST merge designed
//! in `.context/smart-merge.md` (Tier 2). It does **one** thing: parse a
//! file's bytes with the right tree-sitter grammar and reduce it to its
//! ordered list of top-level items, each carrying
//!
//! - a **stable identity key** for later 3-way matching (an import is
//!   keyed by its normalized text; a named declaration by `kind + name`;
//!   anything unnamed falls back to `kind + normalized-text`), and
//! - its **original byte span** (so reconstruction can splice exact
//!   source back, including a declaration's attached leading doc-comments).
//!
//! It deliberately does **not** merge or reconstruct anything — the
//! per-item 3-way classify, ordering policy, and `auto_resolve_conflicts`
//! wiring are the next task (tsk136). The spike that proved this is
//! tractable lives at `crates/oxplow-git/tests/ast_merge_spike.rs`.
//!
//! ## Safety
//!
//! [`parse_top_level_items`] returns `None` when the file isn't a
//! supported language *or* the parse produced any error/missing node —
//! we never operate on an untrusted tree. Callers treat `None` as "leave
//! git's conflict markers", preserving Tier-2's reduce-only invariant.
//!
//! ## Identity-key fidelity (first slice)
//!
//! Imports and the high-value named declarations (`fn`/function, `struct`,
//! `class`/`interface`/`enum`, `type` alias, `trait`, …) get precise
//! `kind+name` keys, which is what the flagship commutative cases need
//! (both sides add a different import / a different top-level decl). Go
//! methods are additionally disambiguated by their receiver. Declarations
//! whose name sits under a declarator list (Rust/TS `const`/`let`/`var`,
//! Go `type`/`var`/`const` groups) currently fall back to text identity:
//! that still merges correctly when one side *adds* such an item (add is
//! key-independent) and is merely conservative — it refuses rather than
//! mis-merges — when both sides *edit the same* one. Tightening those to
//! declarator-name identity is a tsk136 refinement.

use std::ops::Range;

use tree_sitter::{Language as TsLanguage, Node, Parser};

/// A supported source language for the AST merge front half. A subset of
/// `oxplow-code-metrics`'s language set: the six first-slice languages
/// from `.context/smart-merge.md` (rust, typescript, tsx, javascript,
/// python, go). Others are intentionally absent — the merge tier simply
/// won't engage for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
}

/// Map a repo-relative (or any) path to its merge language by extension.
/// Mirrors `oxplow_code_metrics::spec::language_for_path` but restricted
/// to the languages the merge front half supports. `None` ⇒ unsupported,
/// so the AST tier is skipped and git's markers stand.
pub fn language_for_path(path: &str) -> Option<Language> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "rs" => Language::Rust,
        "ts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
        "py" => Language::Python,
        "go" => Language::Go,
        _ => return None,
    })
}

/// Per-language table of the tree-sitter node names that drive identity.
/// Modeled on `oxplow-code-metrics`'s `LanguageSpec` kind tables, but
/// scoped to what top-level-item identity needs rather than complexity
/// metrics.
pub struct MergeSpec {
    /// Node kinds whose identity is their normalized text — imports/use.
    /// A `use`/`import` *is* what it brings into scope, so two textually
    /// distinct imports are distinct items regardless of order.
    pub import_kinds: &'static [&'static str],
    /// "See-through" wrapper kinds (e.g. TS `export_statement`, Python
    /// `decorated_definition`): identity is computed from the declaration
    /// they wrap, while the wrapper's own text/span is still kept so the
    /// `export `/decorator prefix travels with the item.
    pub transparent_kinds: &'static [&'static str],
    /// Field names tried (in order) to descend a transparent wrapper to
    /// the declaration it wraps.
    pub inner_decl_fields: &'static [&'static str],
    /// Field names tried (in order) to find a declaration's identifier.
    pub name_fields: &'static [&'static str],
    /// Loader for the bundled tree-sitter grammar.
    grammar: fn() -> TsLanguage,
}

impl MergeSpec {
    fn tree_sitter_language(&self) -> TsLanguage {
        (self.grammar)()
    }
}

impl Language {
    pub fn spec(&self) -> &'static MergeSpec {
        match self {
            Language::Rust => &RUST,
            Language::TypeScript => &TYPESCRIPT,
            Language::Tsx => &TSX,
            Language::JavaScript => &JAVASCRIPT,
            Language::Python => &PYTHON,
            Language::Go => &GO,
        }
    }
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
pub fn parse_top_level_items(src: &str, lang: Language) -> Option<Vec<Item>> {
    let spec = lang.spec();
    let mut parser = Parser::new();
    parser.set_language(&spec.tree_sitter_language()).ok()?;
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
        let mut start = child.start_byte();
        if let Some((cstart, cend)) = comment_run {
            let gap = &src[cend..child.start_byte()];
            if !gap.contains("\n\n") {
                start = cstart;
            }
        }
        comment_run = None;

        let byte_span = start..child.end_byte();
        let text = src[byte_span.clone()].to_string();
        let key = item_key(child, src, spec);
        items.push(Item {
            key,
            text,
            byte_span,
        });
    }

    Some(items)
}

/// Compute an item's identity key per the spec's rules.
fn item_key(node: Node, src: &str, spec: &MergeSpec) -> String {
    let kind = node.kind();

    // Imports/use: identity is the normalized text.
    if spec.import_kinds.contains(&kind) {
        return format!("import::{}", normalize_ws(&node_text(node, src)));
    }

    // See through wrappers (export/decorated) to the inner declaration
    // for naming, but key off the *inner* kind+name.
    let named = if spec.transparent_kinds.contains(&kind) {
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

// ---- Language specs ----

static RUST: MergeSpec = MergeSpec {
    import_kinds: &["use_declaration", "extern_crate_declaration"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &["name"],
    grammar: || tree_sitter_rust::LANGUAGE.into(),
};

// TypeScript / TSX / JavaScript share identity rules; only the grammar
// (and thus the top-level item kinds the parser yields) differs. A
// top-level `export ...` is an `export_statement` wrapping the real
// declaration under the `declaration` field.
static TS_IMPORT_KINDS: &[&str] = &["import_statement"];
static TS_TRANSPARENT_KINDS: &[&str] = &["export_statement"];
static TS_INNER_DECL_FIELDS: &[&str] = &["declaration"];

static TYPESCRIPT: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
};

static TSX: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
};

static JAVASCRIPT: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    grammar: || tree_sitter_javascript::LANGUAGE.into(),
};

static PYTHON: MergeSpec = MergeSpec {
    import_kinds: &[
        "import_statement",
        "import_from_statement",
        "future_import_statement",
    ],
    // A decorated def wraps the real `function_definition`/`class_definition`
    // under the `definition` field; the decorators ride along in the text.
    transparent_kinds: &["decorated_definition"],
    inner_decl_fields: &["definition"],
    name_fields: &["name"],
    grammar: || tree_sitter_python::LANGUAGE.into(),
};

static GO: MergeSpec = MergeSpec {
    import_kinds: &["import_declaration"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &["name"],
    grammar: || tree_sitter_go::LANGUAGE.into(),
};

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn language_for_path_maps_supported_extensions() {
        assert_eq!(language_for_path("a/b.rs"), Some(Language::Rust));
        assert_eq!(language_for_path("x.ts"), Some(Language::TypeScript));
        assert_eq!(language_for_path("x.tsx"), Some(Language::Tsx));
        assert_eq!(language_for_path("x.js"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.mjs"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.jsx"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.py"), Some(Language::Python));
        assert_eq!(language_for_path("x.go"), Some(Language::Go));
        // Unsupported (or grammar exists in metrics but not the merge slice).
        assert_eq!(language_for_path("x.java"), None);
        assert_eq!(language_for_path("x.txt"), None);
        assert_eq!(language_for_path("noext"), None);
    }

    #[test]
    fn rust_extracts_top_level_items_with_keys() {
        let src = "use std::fmt;\nextern crate foo;\nfn a() {}\nstruct S;\nenum E { A }\n\
                   const K: u8 = 1;\ntrait T {}\nimpl S {}\n";
        let items = parse_top_level_items(src, Language::Rust).unwrap();
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
        let items = parse_top_level_items(src, Language::Rust).unwrap();
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
        assert!(parse_top_level_items("fn a() { ", Language::Rust).is_none());
    }

    #[test]
    fn typescript_imports_classes_and_exported_decls() {
        let src = "import { x } from 'm';\nfunction f() {}\nclass C {}\n\
                   interface I {}\ntype T = number;\nenum E { A }\n\
                   export function g() {}\nexport class D {}\nconst k = 1;\n";
        let items = parse_top_level_items(src, Language::TypeScript).unwrap();
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
        let items = parse_top_level_items(src, Language::Tsx).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import React from 'react';"));
        assert!(k.contains(&"function_declaration::App"));
    }

    #[test]
    fn javascript_imports_and_functions() {
        let src = "import x from 'm';\nfunction f() {}\nclass C {}\nexport function g() {}\n";
        let items = parse_top_level_items(src, Language::JavaScript).unwrap();
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
        let items = parse_top_level_items(src, Language::Python).unwrap();
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
        assert!(parse_top_level_items("def f(:\n", Language::Python).is_none());
    }

    #[test]
    fn go_package_imports_funcs_methods_and_types() {
        let src = "package main\n\
                   import \"fmt\"\n\
                   func F() {}\n\
                   func (s S) M() {}\n\
                   type S struct{}\n\
                   const K = 1\n";
        let items = parse_top_level_items(src, Language::Go).unwrap();
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
    fn empty_source_yields_no_items() {
        assert!(parse_top_level_items("", Language::Rust)
            .unwrap()
            .is_empty());
        assert!(parse_top_level_items("\n\n", Language::Python)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ordering_is_source_order() {
        let src = "fn first() {}\nfn second() {}\nfn third() {}\n";
        let items = parse_top_level_items(src, Language::Rust).unwrap();
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
}
