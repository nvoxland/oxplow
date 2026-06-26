//! Native, in-process per-function code metrics built directly on
//! tree-sitter. Walks the AST and emits per-function records:
//!
//! - **complexity** — cyclomatic complexity number (CCN). Counts
//!   decision-point nodes inside each function body.
//! - **function-length** — line count of the function body.
//! - **parameter-count** — number of declared parameters.
//!
//! Languages bundled: Rust, TypeScript, TSX, JavaScript, Python, Go,
//! Java, C, C++. Adding a new language is one entry in `Language`
//! plus a `LanguageSpec` with the relevant tree-sitter node names.
//!
//! Files in unsupported languages are silently skipped (the caller
//! sees no findings for them).

use std::path::Path;

use tree_sitter::Node;

mod ast;
mod spec;

pub use ast::{ast_query, parse, query, AstQueryError, QueryMatch};
pub use spec::{
    language_for_path, language_from_lsp_id, language_from_name, Language, LanguageSpec,
    VisibilityStrategy,
};
pub use tree_sitter::Tree;

/// Coarse public/private classification. Heuristic per language —
/// see the comment on `LanguageSpec::visibility` for the strategy
/// table. The frontend uses this to drive a "Show private" filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
    Unknown,
}

/// One per-function metric record. The caller is responsible for
/// fanning this out into the three downstream finding kinds
/// (complexity / function-length / parameter-count).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionMetrics {
    /// Repo-relative file path (caller-supplied; pass-through).
    pub path: String,
    /// Best-effort function name. May include receiver/class qualifier.
    pub name: String,
    /// Cyclomatic complexity (decision points + 1).
    pub complexity: u32,
    /// Body line count (end_line - start_line + 1).
    pub length: u32,
    /// Declared parameter count.
    pub parameter_count: u32,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line.
    pub end_line: u32,
    /// Outer-to-inner names of the named-declaration ancestors this
    /// function lives inside (class / impl / module / namespace).
    /// Empty for top-level functions. Language-specific — see
    /// `LanguageSpec::container_kinds`.
    pub container_path: Vec<String>,
    /// Coarse public/private classification (heuristic per language).
    pub visibility: Visibility,
}

/// Analyze a single file. Returns an empty Vec for unsupported
/// languages (or files that fail to parse).
pub fn analyze_file(path: &str, source: &str) -> Vec<FunctionMetrics> {
    let Some(lang) = language_for_path(path) else {
        return Vec::new();
    };
    analyze_with_language(path, source, lang)
}

/// Like `analyze_file` but with the language explicitly chosen
/// (useful when the path doesn't reveal the language, e.g. content
/// fetched from a git ref into a temp buffer).
pub fn analyze_with_language(path: &str, source: &str, language: Language) -> Vec<FunctionMetrics> {
    let Some(tree) = parse(source, language) else {
        return Vec::new();
    };
    let spec = language.spec();
    let mut out = Vec::new();
    walk_functions(tree.root_node(), source.as_bytes(), spec, path, &mut out);
    out
}

/// A code marker (TODO / FIXME / …) found inside a comment — the
/// language-agnostic counterpart of [`FunctionMetrics`] for the
/// `oxplow.todos` metric. The language layer owns "what is a comment"
/// (every tree-sitter grammar tags comment nodes with a kind containing
/// "comment"), so the metric never special-cases a language.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// 1-based line the marker sits on.
    pub line: u32,
    /// The marker keyword that matched (`TODO`, `FIXME`, …).
    pub kind: String,
    /// The marker's comment line, cleaned of leading comment delimiters.
    pub text: String,
}

/// The marker keywords scanned for, in priority order.
const MARKER_KEYWORDS: &[&str] = &["TODO", "FIXME", "HACK", "XXX", "BUG"];

/// Scan a source file for comment markers (TODO / FIXME / HACK / XXX /
/// BUG). Comment-aware: only text inside actual comment nodes is
/// considered (so a `"TODO"` string literal doesn't count), using the
/// language's own grammar. Returns an empty Vec for unsupported /
/// unparseable input.
pub fn markers(source: &str, language: Language) -> Vec<Marker> {
    let Some(tree) = parse(source, language) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_markers(tree.root_node(), source.as_bytes(), &mut out);
    out.sort_by_key(|m| m.line);
    out
}

fn collect_markers(node: Node<'_>, src: &[u8], out: &mut Vec<Marker>) {
    // Every grammar tags comments with a kind containing "comment"
    // (`line_comment` / `block_comment` / `comment`); match generically.
    if node.kind().contains("comment") {
        let text = node.utf8_text(src).unwrap_or("");
        let base = node.start_position().row as u32;
        for (i, line) in text.lines().enumerate() {
            if let Some(kw) = first_marker(line) {
                out.push(Marker {
                    line: base + i as u32 + 1,
                    kind: kw.to_string(),
                    text: clean_comment_line(line),
                });
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_markers(child, src, out);
    }
}

/// The first marker keyword appearing as a standalone token in `line`.
fn first_marker(line: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for kw in MARKER_KEYWORDS {
        if let Some(pos) = find_token(line, kw) {
            if best.map(|(p, _)| pos < p).unwrap_or(true) {
                best = Some((pos, kw));
            }
        }
    }
    best.map(|(_, kw)| kw)
}

/// Byte index of `needle` in `hay` bounded by non-word chars on both
/// sides (so `TODONE` / `mastodon` don't match `TODO`).
fn find_token(hay: &str, needle: &str) -> Option<usize> {
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let mut i = 0;
    while i + nb.len() <= hb.len() {
        if &hb[i..i + nb.len()] == nb {
            let before_ok = i == 0 || !is_word(hb[i - 1]);
            let after_ok = i + nb.len() == hb.len() || !is_word(hb[i + nb.len()]);
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Trim a comment line of its leading comment delimiters / whitespace
/// (`//`, `/*`, `*`, `;`, `#`) for display.
fn clean_comment_line(line: &str) -> String {
    line.trim()
        .trim_start_matches(['/', '*', ';', '#', ' ', '\t'])
        .trim()
        .to_string()
}

/// Recursive function-finder. When we hit a node whose kind matches
/// one of `spec.function_kinds` (or, for grammars with form-head
/// matchers, a `list_lit` whose head sym is in
/// `spec.function_form_heads`), compute its metrics. We always
/// descend so nested closures / methods get their own records.
fn walk_functions(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    path: &str,
    out: &mut Vec<FunctionMetrics>,
) {
    if is_function_node(node, src, spec) {
        if let Some(rec) = function_metrics(node, src, spec, path) {
            out.push(rec);
        }
        // Still descend so we capture nested closures / methods.
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_functions(child, src, spec, path, out);
    }
}

/// Whether `node` should be treated as a function root. Either
/// `node.kind()` is in `spec.function_kinds`, or — for grammars
/// with a generic list node like Clojure's `list_lit` — the head
/// symbol's text is in `spec.function_form_heads`.
pub fn is_function_node(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> bool {
    if spec.function_kinds.contains(&node.kind()) {
        return true;
    }
    if !spec.function_form_heads.is_empty() && node.kind() == "list_lit" {
        if let Some(head) = head_symbol_text(node, src) {
            return spec.function_form_heads.contains(&head);
        }
    }
    false
}

/// Same shape as `is_function_node` but for decision points.
fn is_decision_node(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> bool {
    if spec.decision_kinds.contains(&node.kind()) {
        return true;
    }
    if !spec.decision_form_heads.is_empty() && node.kind() == "list_lit" {
        if let Some(head) = head_symbol_text(node, src) {
            return spec.decision_form_heads.contains(&head);
        }
    }
    false
}

/// Read the text of the first significant symbol child of a
/// `list_lit` node. Skips comments. Returns `None` if the first
/// significant child isn't a `sym_lit`. tree-sitter-clojure
/// represents the actual symbol identifier as a `sym_name` child
/// of `sym_lit` (with any leading `^…` metadata as a sibling
/// `meta_lit` child of the same sym_lit), so we read sym_name's
/// text rather than sym_lit's full text.
pub fn head_symbol_text<'a>(node: Node<'_>, src: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "comment" => continue,
            "sym_lit" => {
                return sym_lit_name(child, src);
            }
            _ => return None,
        }
    }
    None
}

/// Extract the symbol name from a `sym_lit` by reading its
/// `sym_name` child. Falls back to the sym_lit's own text when
/// the grammar shape differs.
fn sym_lit_name<'a>(sym_lit: Node<'_>, src: &'a [u8]) -> Option<&'a str> {
    let mut cursor = sym_lit.walk();
    for child in sym_lit.named_children(&mut cursor) {
        if child.kind() == "sym_name" {
            return child.utf8_text(src).ok();
        }
    }
    sym_lit.utf8_text(src).ok()
}

fn function_metrics(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    path: &str,
) -> Option<FunctionMetrics> {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let length = end_line.saturating_sub(start_line) + 1;

    let name = function_name(node, src, spec).unwrap_or_else(|| "(anonymous)".into());
    let parameter_count = count_parameters(node, src, spec);
    let complexity = count_decision_points(node, src, spec) + 1;
    let container_path = container_path(node, src, spec);
    let visibility = visibility_for(node, src, spec, &name);

    Some(FunctionMetrics {
        path: path.into(),
        name,
        complexity,
        length,
        parameter_count,
        start_line,
        end_line,
        container_path,
        visibility,
    })
}

fn visibility_for(node: Node<'_>, src: &[u8], spec: &LanguageSpec, name: &str) -> Visibility {
    match spec.visibility {
        VisibilityStrategy::Unknown => Visibility::Unknown,
        VisibilityStrategy::RustModifier => rust_visibility(node),
        VisibilityStrategy::TsClassModifier => ts_visibility(node, src),
        VisibilityStrategy::JavaModifier => java_visibility(node, src),
        VisibilityStrategy::CppAccessSpecifier => cpp_visibility(node, src),
        VisibilityStrategy::GoCapitalization => go_visibility(name),
        VisibilityStrategy::PythonUnderscore => python_visibility(name),
        VisibilityStrategy::CStatic => c_visibility(node, src),
        VisibilityStrategy::ClojureForm => clojure_visibility(node, src),
    }
}

fn clojure_visibility(node: Node<'_>, src: &[u8]) -> Visibility {
    // `defn-` is the explicit private form.
    if let Some(head) = head_symbol_text(node, src) {
        if head == "defn-" {
            return Visibility::Private;
        }
    }
    // `^:private` metadata: tree-sitter-clojure attaches metadata
    // as a child of the SYM_LIT it annotates, not as a sibling on
    // the list_lit. So `(defn ^:private foo …)` puts the meta_lit
    // inside `foo`'s sym_lit. Walk every sym_lit child of the
    // list_lit and check for a meta_lit/old_meta_lit child whose
    // text mentions `:private`.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "sym_lit" {
            continue;
        }
        let mut inner = child.walk();
        for meta in child.named_children(&mut inner) {
            if matches!(meta.kind(), "meta_lit" | "old_meta_lit") {
                if let Ok(text) = meta.utf8_text(src) {
                    if text.contains(":private") {
                        return Visibility::Private;
                    }
                }
            }
        }
    }
    Visibility::Public
}

fn rust_visibility(node: Node<'_>) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return Visibility::Public;
        }
    }
    // Closures + items without `pub` → module-private.
    if matches!(node.kind(), "function_item" | "function_signature_item") {
        return Visibility::Private;
    }
    Visibility::Unknown
}

fn ts_visibility(node: Node<'_>, src: &[u8]) -> Visibility {
    // Method inside a class: look for `accessibility_modifier` child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "accessibility_modifier" {
            if let Ok(text) = child.utf8_text(src) {
                if text == "private" || text == "protected" {
                    return Visibility::Private;
                }
                if text == "public" {
                    return Visibility::Public;
                }
            }
        }
    }
    // JS/TS class field/method names starting with `#` are hard-private.
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(text) = name_node.utf8_text(src) {
            if text.starts_with('#') {
                return Visibility::Private;
            }
        }
    }
    // If this method sits inside a class, default = public.
    if has_ancestor_kind(
        node,
        &["class_declaration", "abstract_class_declaration", "class"],
    ) {
        return Visibility::Public;
    }
    // Top-level function: check whether an enclosing
    // `export_statement` covers this declaration.
    if has_ancestor_kind(node, &["export_statement"]) {
        return Visibility::Public;
    }
    // Otherwise it's a non-exported top-level function (or arrow
    // assigned to a const without export) → file-private.
    Visibility::Private
}

fn java_visibility(node: Node<'_>, _src: &[u8]) -> Visibility {
    // `modifiers` child (if any) groups every declaration modifier.
    // Look up the literal `private` / `protected` / `public` keyword
    // by walking that node's children.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner = child.walk();
            for m in child.children(&mut inner) {
                let kind = m.kind();
                if kind == "private" || kind == "protected" {
                    return Visibility::Private;
                }
                if kind == "public" {
                    return Visibility::Public;
                }
            }
        }
    }
    // Java default (package-private) — treat as Private since it's
    // not generally callable from outside the package.
    Visibility::Private
}

fn cpp_visibility(node: Node<'_>, src: &[u8]) -> Visibility {
    // C++ function definitions outside any class are file/scope
    // public (no class access spec applies).
    let mut current = node.parent();
    let mut class_kind: Option<&str> = None;
    while let Some(parent) = current {
        let kind = parent.kind();
        if matches!(kind, "class_specifier" | "struct_specifier") {
            class_kind = Some(kind);
            break;
        }
        current = parent.parent();
    }
    let Some(class_kind) = class_kind else {
        return Visibility::Public;
    };
    // Default visibility: private inside class, public inside struct.
    let default_vis = if class_kind == "class_specifier" {
        Visibility::Private
    } else {
        Visibility::Public
    };
    // Walk the class body looking for the most recent
    // access_specifier preceding `node`.
    let Some(class_node) = current else {
        return default_vis;
    };
    let body = class_node.child_by_field_name("body").unwrap_or(class_node);
    let target_start = node.start_byte();
    let mut cursor = body.walk();
    let mut current_vis = default_vis;
    for child in body.children(&mut cursor) {
        if child.start_byte() > target_start {
            break;
        }
        if child.kind() == "access_specifier" {
            if let Ok(text) = child.utf8_text(src) {
                let trimmed = text.trim_end_matches(':').trim();
                current_vis = match trimmed {
                    "public" => Visibility::Public,
                    "private" => Visibility::Private,
                    "protected" => Visibility::Private,
                    _ => current_vis,
                };
            }
        }
    }
    current_vis
}

fn go_visibility(name: &str) -> Visibility {
    let first = name.chars().next();
    match first {
        Some(c) if c.is_uppercase() => Visibility::Public,
        Some(c) if c.is_lowercase() => Visibility::Private,
        _ => Visibility::Unknown,
    }
}

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

fn c_visibility(node: Node<'_>, src: &[u8]) -> Visibility {
    // Look for a `storage_class_specifier` whose text is `static`
    // among the function definition's children. Tree-sitter-c puts
    // it directly on the function_definition node.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "storage_class_specifier" {
            if let Ok(text) = child.utf8_text(src) {
                if text == "static" {
                    return Visibility::Private;
                }
            }
        }
    }
    Visibility::Public
}

fn has_ancestor_kind(node: Node<'_>, kinds: &[&str]) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if kinds.contains(&parent.kind()) {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Walk parents of `node` and collect outer-to-inner names of every
/// ancestor whose kind is in `spec.container_kinds`. Stops at the
/// nearest enclosing function — a closure inside a method does not
/// inherit the method's class as a container (the method itself is
/// already its own record with that container_path).
fn container_path(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        if spec.function_kinds.contains(&parent.kind()) {
            break;
        }
        if spec.container_kinds.contains(&parent.kind()) {
            if let Some(name) = container_name(parent, src, spec) {
                names.push(name);
            }
        }
        current = parent.parent();
    }
    names.reverse();
    names
}

fn container_name(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> Option<String> {
    for field in spec.container_name_fields {
        if let Some(name_node) = node.child_by_field_name(field) {
            let leaf = innermost_identifier(name_node).unwrap_or(name_node);
            if let Ok(text) = leaf.utf8_text(src) {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn function_name(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> Option<String> {
    // Clojure: list_lit shaped as (defn name [args] …). Head sym
    // is the form (`defn`/`defmacro`/etc.); name is the next
    // sym_lit child (skipping comments + metadata).
    if !spec.function_form_heads.is_empty() && node.kind() == "list_lit" {
        return clojure_function_name(node, src);
    }
    for field in spec.name_fields {
        if let Some(name_node) = node.child_by_field_name(field) {
            // C/C++ functions store the name inside a nested
            // `function_declarator` subtree, not directly on the
            // declarator field. Descend until we find a leaf
            // identifier so we report `foo` rather than the whole
            // `foo(int x)` declarator text.
            let leaf = innermost_identifier(name_node).unwrap_or(name_node);
            if let Ok(text) = leaf.utf8_text(src) {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Extract the function name from a Clojure `list_lit`. Skips the
/// head form symbol and any leading comments. The actual
/// identifier text is the `sym_name` child of the second sym_lit
/// (the first sym_lit is the form head like `defn`). Anonymous
/// `(fn …)` returns None — caller falls back to "(anonymous)".
fn clojure_function_name(node: Node<'_>, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let mut saw_head = false;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "comment" => continue,
            "sym_lit" => {
                if !saw_head {
                    saw_head = true;
                    continue;
                }
                return sym_lit_name(child, src).map(|s| s.to_string());
            }
            _ => return None,
        }
    }
    None
}

/// Walk down a declarator subtree to the innermost identifier-like
/// node. Used by C/C++ where the `declarator` field is a composite.
fn innermost_identifier<'a>(node: Node<'a>) -> Option<Node<'a>> {
    if matches!(
        node.kind(),
        "identifier" | "field_identifier" | "type_identifier" | "destructor_name" | "operator_name"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = innermost_identifier(child) {
            return Some(found);
        }
    }
    None
}

fn count_parameters(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> u32 {
    // Clojure: param vector is the first vec_lit child after the
    // head + name (or just after the head for `(fn [x] …)`).
    if !spec.function_form_heads.is_empty() && node.kind() == "list_lit" {
        return clojure_count_parameters(node, src);
    }
    // First try a direct field lookup; falls through to a recursive
    // descendant search for grammars (C/C++) where the parameter list
    // is nested inside a `function_declarator` rather than reachable
    // via a field on the function node itself.
    let params_node = spec
        .param_list_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .or_else(|| find_parameter_list(node));
    let Some(params_node) = params_node else {
        return 0;
    };
    let mut cursor = params_node.walk();
    let mut count = 0u32;
    for child in params_node.named_children(&mut cursor) {
        if spec.parameter_kinds.contains(&child.kind()) {
            count += 1;
        }
    }
    count
}

fn clojure_count_parameters(node: Node<'_>, _src: &[u8]) -> u32 {
    // Walk the list_lit looking for the first vec_lit. Count its
    // direct named children minus metadata. This is approximate —
    // destructuring like `[{:keys [a b]} c]` counts the map as 1
    // and `c` as 1, which matches what most tooling does.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "vec_lit" {
            let mut inner = child.walk();
            let mut count = 0u32;
            for p in child.named_children(&mut inner) {
                if matches!(p.kind(), "comment" | "meta_lit" | "old_meta_lit") {
                    continue;
                }
                count += 1;
            }
            return count;
        }
    }
    0
}

/// Recursively look for a `parameter_list` node in a function's
/// declarator subtree. Used for C/C++ where the structure is
/// `function_definition > declarator(function_declarator) > parameters(parameter_list)`.
fn find_parameter_list<'a>(node: Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "parameter_list" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_parameter_list(child) {
            return Some(found);
        }
    }
    None
}

fn count_decision_points(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> u32 {
    // We never want to count the function node itself (function kinds
    // never appear in `decision_kinds` anyway), so just descend into
    // children and tally there.
    let mut count = 0u32;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, src, spec, node.id());
    }
    count
}

fn count_decision_subtree(node: Node<'_>, src: &[u8], spec: &LanguageSpec, root_id: usize) -> u32 {
    let mut count = 0u32;
    if node.id() != root_id && is_decision_node(node, src, spec) {
        count += 1;
    }
    // Stop descending into nested function bodies — their decisions
    // belong to that function's own metrics record. (Don't stop at
    // the root function itself, which is the same node id we
    // started from.)
    if node.id() != root_id && is_function_node(node, src, spec) {
        return count;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, src, spec, root_id);
    }
    count
}

/// Convenience: peek at a path's extension to decide whether we
/// support it. Useful for callers that want to skip read-from-disk
/// when the language isn't supported anyway.
pub fn is_supported_path(path: impl AsRef<Path>) -> bool {
    let p = path.as_ref().to_string_lossy();
    language_for_path(&p).is_some()
}

#[cfg(test)]
mod tests;
