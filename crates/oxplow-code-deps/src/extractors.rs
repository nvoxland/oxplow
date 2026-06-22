//! Per-language tree-walkers. Each `walk_<lang>` recurses through the
//! AST looking for the language's import-shaped nodes and pushes one
//! [`ImportEdge`] per declaration. Module strings are normalized but
//! never *resolved* — `"std::fs"`, `"./Foo"`, `"<stdio.h>"` all flow
//! through verbatim.

use oxplow_code_metrics::Language;
use tree_sitter::Node;

use crate::{lines, ImportEdge, ImportKind};

pub(crate) fn walk(
    root: Node<'_>,
    src: &[u8],
    language: Language,
    path: &str,
    out: &mut Vec<ImportEdge>,
) {
    match language {
        Language::Rust => walk_rust(root, src, path, out),
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            walk_js_like(root, src, path, out)
        }
        Language::Python => walk_python(root, src, path, out),
        Language::Go => walk_go(root, src, path, out),
        Language::Java => walk_java(root, src, path, out),
        Language::C => walk_c_like(root, src, path, out, /*cpp=*/ false),
        Language::Cpp => walk_c_like(root, src, path, out, /*cpp=*/ true),
        Language::Clojure => walk_clojure(root, src, path, out),
        Language::CSharp => walk_csharp(root, src, path, out),
    }
}

// ---- C# ----

fn walk_csharp(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    if node.kind() == "using_directive" {
        // `using System.Text;` / `using static Foo.Bar;` / `using Alias = Foo;`
        // — take the first qualified/identifier child as the namespace. The
        // alias target (RHS of `=`) is also a name child; the first one is the
        // namespace being referenced, which is what matters for the zone graph.
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if matches!(child.kind(), "qualified_name" | "identifier") {
                if let Ok(text) = child.utf8_text(src) {
                    let (start, end) = lines(node);
                    out.push(ImportEdge {
                        from_path: path.into(),
                        raw: node_text(node, src),
                        module: text.trim().to_string(),
                        kind: ImportKind::Using,
                        start_line: start,
                        end_line: end,
                    });
                }
                break;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_csharp(child, src, path, out);
    }
}

// ---- Rust ----

fn walk_rust(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    let kind = node.kind();
    match kind {
        "use_declaration" => {
            // `use foo::bar::{a, b};` — emit one edge per leaf path,
            // common-prefixing the grouped tail. Simpler: emit the
            // full source text as `raw` and the leading path before
            // the `{` (or whole path) as `module`. Grouping isn't
            // structurally important for zone-edge detection — what
            // matters is which top-level crate is reached.
            let raw = node_text(node, src);
            for module in rust_use_modules(node, src) {
                let (start, end) = lines(node);
                out.push(ImportEdge {
                    from_path: path.into(),
                    raw: raw.clone(),
                    module,
                    kind: ImportKind::Use,
                    start_line: start,
                    end_line: end,
                });
            }
        }
        "extern_crate_declaration" => {
            // `extern crate foo;` — the name child is the crate name.
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
            {
                let (start, end) = lines(node);
                out.push(ImportEdge {
                    from_path: path.into(),
                    raw: node_text(node, src),
                    module: name.to_string(),
                    kind: ImportKind::Use,
                    start_line: start,
                    end_line: end,
                });
            }
        }
        _ => {}
    }
    // Descend so nested modules (`mod foo { use ... }`) are covered.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust(child, src, path, out);
    }
}

/// Decompose a `use_declaration` into top-level module references.
/// `use foo::bar;` → `["foo::bar"]`.
/// `use foo::{a, b};` → `["foo::a", "foo::b"]`.
/// `use foo::*;` → `["foo"]` (wildcard collapses to the prefix).
/// `use foo as bar;` → `["foo"]` (the alias doesn't matter for
/// zone-graph purposes).
fn rust_use_modules(use_node: Node<'_>, src: &[u8]) -> Vec<String> {
    // The single named child of a use_declaration is the path tree:
    // scoped_identifier, scoped_use_list, use_as_clause, use_wildcard,
    // identifier, or use_list.
    let mut cursor = use_node.walk();
    for child in use_node.named_children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            continue;
        }
        return expand_rust_use(child, src, "");
    }
    Vec::new()
}

fn expand_rust_use(node: Node<'_>, src: &[u8], prefix: &str) -> Vec<String> {
    let join = |suffix: &str| {
        if prefix.is_empty() {
            suffix.to_string()
        } else if suffix.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix}::{suffix}")
        }
    };
    match node.kind() {
        "self" => {
            // `use foo::{self, bar}` — `self` collapses to the parent
            // path. Emit just the prefix.
            if prefix.is_empty() {
                vec!["self".to_string()]
            } else {
                vec![prefix.to_string()]
            }
        }
        "identifier" | "crate" | "super" | "metavariable" => {
            vec![join(node.utf8_text(src).unwrap_or("").trim())]
        }
        "scoped_identifier" => {
            // path::name — emit `prefix::path::name`.
            let text = node.utf8_text(src).unwrap_or("").trim();
            vec![join(text)]
        }
        "scoped_use_list" => {
            // path::{...} — recurse into the list with extended prefix.
            let path_text = node
                .child_by_field_name("path")
                .and_then(|n| n.utf8_text(src).ok())
                .unwrap_or("")
                .trim();
            let new_prefix = if prefix.is_empty() {
                path_text.to_string()
            } else if path_text.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}::{path_text}")
            };
            if let Some(list) = node.child_by_field_name("list") {
                expand_rust_use(list, src, &new_prefix)
            } else {
                vec![new_prefix]
            }
        }
        "use_list" => {
            let mut out = Vec::new();
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                out.extend(expand_rust_use(child, src, prefix));
            }
            out
        }
        "use_as_clause" => {
            // `path as alias` — keep the path, drop the alias.
            if let Some(path) = node.child_by_field_name("path") {
                expand_rust_use(path, src, prefix)
            } else {
                Vec::new()
            }
        }
        "use_wildcard" => {
            // `path::*` — collapse to the prefix path.
            let path_text = node
                .named_child(0)
                .and_then(|n| n.utf8_text(src).ok())
                .unwrap_or("")
                .trim();
            vec![join(path_text)]
        }
        _ => {
            // Fall back to raw text for unknown shapes — keeps the
            // edge present rather than silently dropping it.
            let text = node.utf8_text(src).unwrap_or("").trim();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![join(text)]
            }
        }
    }
}

// ---- JS / TS / TSX ----

fn walk_js_like(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    match node.kind() {
        "import_statement" => {
            // `source` field holds the `"foo"` string literal.
            if let Some(source) = node.child_by_field_name("source") {
                if let Some(module) = strip_string_quotes(source.utf8_text(src).unwrap_or("")) {
                    let (start, end) = lines(node);
                    out.push(ImportEdge {
                        from_path: path.into(),
                        raw: node_text(node, src),
                        module,
                        kind: ImportKind::Import,
                        start_line: start,
                        end_line: end,
                    });
                }
            }
        }
        "export_statement" => {
            // `export { x } from "foo";` / `export * from "foo";`.
            if let Some(source) = node.child_by_field_name("source") {
                if let Some(module) = strip_string_quotes(source.utf8_text(src).unwrap_or("")) {
                    let (start, end) = lines(node);
                    out.push(ImportEdge {
                        from_path: path.into(),
                        raw: node_text(node, src),
                        module,
                        kind: ImportKind::Import,
                        start_line: start,
                        end_line: end,
                    });
                }
            }
        }
        "call_expression" => {
            // `require("foo")` — function `require`, single string arg.
            if let Some(function) = node.child_by_field_name("function") {
                if function.utf8_text(src).ok() == Some("require") {
                    if let Some(args) = node.child_by_field_name("arguments") {
                        if let Some(first) = args.named_child(0) {
                            if matches!(first.kind(), "string" | "template_string") {
                                if let Some(module) =
                                    strip_string_quotes(first.utf8_text(src).unwrap_or(""))
                                {
                                    let (start, end) = lines(node);
                                    out.push(ImportEdge {
                                        from_path: path.into(),
                                        raw: node_text(node, src),
                                        module,
                                        kind: ImportKind::Import,
                                        start_line: start,
                                        end_line: end,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_js_like(child, src, path, out);
    }
}

// ---- Python ----

fn walk_python(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    match node.kind() {
        "import_statement" => {
            // `import foo` / `import foo.bar as baz` / `import a, b`.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                if let Some(module) = python_dotted(child, src) {
                    let (start, end) = lines(node);
                    out.push(ImportEdge {
                        from_path: path.into(),
                        raw: node_text(node, src),
                        module,
                        kind: ImportKind::PyImport,
                        start_line: start,
                        end_line: end,
                    });
                }
            }
        }
        "import_from_statement" => {
            // `from foo import bar` — module is the `module_name` child
            // (could be relative: `relative_import` node, e.g. `.foo`).
            let module_node = node.child_by_field_name("module_name").or_else(|| {
                // tree-sitter-python sometimes exposes module via the
                // first named child rather than a field, depending on
                // grammar version. Probe for `dotted_name` /
                // `relative_import` directly.
                let mut c = node.walk();
                let mut found: Option<Node<'_>> = None;
                for child in node.named_children(&mut c) {
                    if matches!(
                        child.kind(),
                        "dotted_name" | "relative_import" | "aliased_import"
                    ) {
                        found = Some(child);
                        break;
                    }
                }
                found
            });
            if let Some(m) = module_node {
                if let Some(module) = python_dotted(m, src) {
                    let (start, end) = lines(node);
                    out.push(ImportEdge {
                        from_path: path.into(),
                        raw: node_text(node, src),
                        module,
                        kind: ImportKind::PyImport,
                        start_line: start,
                        end_line: end,
                    });
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python(child, src, path, out);
    }
}

fn python_dotted(node: Node<'_>, src: &[u8]) -> Option<String> {
    match node.kind() {
        "dotted_name" => node.utf8_text(src).ok().map(|s| s.trim().to_string()),
        "relative_import" => node.utf8_text(src).ok().map(|s| s.trim().to_string()),
        "aliased_import" => {
            // `foo.bar as baz` — keep the `foo.bar` only.
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
                .map(|s| s.trim().to_string())
        }
        _ => None,
    }
}

// ---- Go ----

fn walk_go(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    if node.kind() == "import_spec" {
        // `path` field holds an `interpreted_string_literal`.
        if let Some(p) = node.child_by_field_name("path") {
            if let Some(module) = strip_string_quotes(p.utf8_text(src).unwrap_or("")) {
                let (start, end) = lines(node);
                out.push(ImportEdge {
                    from_path: path.into(),
                    raw: node_text(node, src),
                    module,
                    kind: ImportKind::GoImport,
                    start_line: start,
                    end_line: end,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go(child, src, path, out);
    }
}

// ---- Java ----

fn walk_java(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    if node.kind() == "import_declaration" {
        // The qualified path is everything between `import` and `;`
        // minus optional `static`. Easiest: take the first
        // `scoped_identifier` / `identifier` / `asterisk` child.
        let mut module_parts: Vec<String> = Vec::new();
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            let kind = child.kind();
            if matches!(kind, "scoped_identifier" | "identifier" | "asterisk") {
                if let Ok(text) = child.utf8_text(src) {
                    module_parts.push(text.trim().to_string());
                }
            }
        }
        // For `import foo.bar.*;` the children are `scoped_identifier`
        // (foo.bar) + `asterisk` — join with `.`. For
        // `import foo.bar.Baz;` it's just the one `scoped_identifier`.
        if !module_parts.is_empty() {
            let module = module_parts.join(".");
            let (start, end) = lines(node);
            out.push(ImportEdge {
                from_path: path.into(),
                raw: node_text(node, src),
                module,
                kind: ImportKind::JavaImport,
                start_line: start,
                end_line: end,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java(child, src, path, out);
    }
}

// ---- C / C++ ----

fn walk_c_like(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>, cpp: bool) {
    match node.kind() {
        "preproc_include" => {
            // `path` child is `system_lib_string` (<…>) or `string_literal` ("…").
            if let Some(p) = node.child_by_field_name("path") {
                let module = p.utf8_text(src).unwrap_or("").trim().to_string();
                let (start, end) = lines(node);
                out.push(ImportEdge {
                    from_path: path.into(),
                    raw: node_text(node, src),
                    module,
                    kind: ImportKind::Include,
                    start_line: start,
                    end_line: end,
                });
            }
        }
        "using_declaration" | "using_directive" if cpp => {
            // `using foo::bar;` / `using namespace foo;`.
            // Most useful module text is the qualified-id child.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                if matches!(
                    child.kind(),
                    "qualified_identifier" | "identifier" | "namespace_identifier"
                ) {
                    if let Ok(text) = child.utf8_text(src) {
                        let (start, end) = lines(node);
                        out.push(ImportEdge {
                            from_path: path.into(),
                            raw: node_text(node, src),
                            module: text.trim().to_string(),
                            kind: ImportKind::Using,
                            start_line: start,
                            end_line: end,
                        });
                        break;
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_c_like(child, src, path, out, cpp);
    }
}

// ---- Clojure ----

fn walk_clojure(node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    if node.kind() == "list_lit" {
        if let Some(head) = head_sym(node, src) {
            match head {
                "ns" => clojure_ns_imports(node, src, path, out),
                "require" | "use" | "import" => {
                    clojure_top_require(node, src, path, out, head);
                }
                _ => {}
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_clojure(child, src, path, out);
    }
}

fn clojure_ns_imports(ns_node: Node<'_>, src: &[u8], path: &str, out: &mut Vec<ImportEdge>) {
    // (ns my.ns (:require [foo.bar :as fb] foo.baz) (:use foo.qux) (:import [java.util Date]))
    // Each clause is a list_lit whose first sym is `:require`/`:use`/`:import`.
    let mut cursor = ns_node.walk();
    for clause in ns_node.named_children(&mut cursor) {
        if clause.kind() != "list_lit" {
            continue;
        }
        // First named child should be a keyword like `:require`.
        let mut clause_cursor = clause.walk();
        let mut keyword: Option<Node<'_>> = None;
        for child in clause.named_children(&mut clause_cursor) {
            if child.kind() == "kwd_lit" {
                keyword = Some(child);
                break;
            }
        }
        let Some(keyword) = keyword else { continue };
        let kw_text = keyword.utf8_text(src).unwrap_or("");
        let clause_kind = match kw_text {
            ":require" | ":require-macros" => "require",
            ":use" => "use",
            ":import" => "import",
            _ => continue,
        };
        // Remaining named children are the specs (sym_lit or vec_lit
        // with sym_lit head).
        let mut inner = clause.walk();
        for spec in clause.named_children(&mut inner) {
            if spec.id() == keyword.id() {
                continue;
            }
            if let Some(module) = clojure_spec_to_module(spec, src, clause_kind) {
                let (start, end) = lines(spec);
                out.push(ImportEdge {
                    from_path: path.into(),
                    raw: node_text(spec, src),
                    module,
                    kind: ImportKind::CljRequire,
                    start_line: start,
                    end_line: end,
                });
            }
        }
    }
}

fn clojure_top_require(
    node: Node<'_>,
    src: &[u8],
    path: &str,
    out: &mut Vec<ImportEdge>,
    head: &str,
) {
    // (require '[foo.bar :as fb]) — args follow the head sym.
    let mut cursor = node.walk();
    let mut saw_head = false;
    for child in node.named_children(&mut cursor) {
        if !saw_head {
            if child.kind() == "sym_lit" {
                saw_head = true;
            }
            continue;
        }
        // The arg is usually `quoting_lit` wrapping a `vec_lit` /
        // `sym_lit`. Descend into it.
        let target = if child.kind() == "quoting_lit" {
            let mut qc = child.walk();
            let mut inner: Option<Node<'_>> = None;
            for n in child.named_children(&mut qc) {
                if matches!(n.kind(), "vec_lit" | "sym_lit") {
                    inner = Some(n);
                    break;
                }
            }
            inner.unwrap_or(child)
        } else {
            child
        };
        if let Some(module) = clojure_spec_to_module(target, src, head) {
            let (start, end) = lines(target);
            out.push(ImportEdge {
                from_path: path.into(),
                raw: node_text(node, src),
                module,
                kind: ImportKind::CljRequire,
                start_line: start,
                end_line: end,
            });
        }
    }
}

fn clojure_spec_to_module(spec: Node<'_>, src: &[u8], _clause_kind: &str) -> Option<String> {
    match spec.kind() {
        "sym_lit" => Some(symbol_text(spec, src)?.to_string()),
        "vec_lit" => {
            // First sym_lit child is the module name.
            let mut c = spec.walk();
            let mut sym: Option<Node<'_>> = None;
            for n in spec.named_children(&mut c) {
                if n.kind() == "sym_lit" {
                    sym = Some(n);
                    break;
                }
            }
            sym.and_then(|n| symbol_text(n, src)).map(|s| s.to_string())
        }
        _ => None,
    }
}

fn head_sym<'a>(node: Node<'_>, src: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "comment" => continue,
            "sym_lit" => return symbol_text(child, src),
            _ => return None,
        }
    }
    None
}

fn symbol_text<'a>(sym_lit: Node<'_>, src: &'a [u8]) -> Option<&'a str> {
    let mut cursor = sym_lit.walk();
    for child in sym_lit.named_children(&mut cursor) {
        if child.kind() == "sym_name" {
            return child.utf8_text(src).ok();
        }
    }
    sym_lit.utf8_text(src).ok()
}

// ---- Shared helpers ----

fn node_text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn strip_string_quotes(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return None;
    }
    let first = trimmed.chars().next()?;
    let last = trimmed.chars().last()?;
    if (first == '"' && last == '"')
        || (first == '\'' && last == '\'')
        || (first == '`' && last == '`')
    {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else {
        Some(trimmed.to_string())
    }
}
