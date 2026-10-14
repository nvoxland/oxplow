//! Native, in-process per-function code metrics built directly on
//! tree-sitter. Walks the AST and emits per-function records:
//!
//! - **complexity** — cyclomatic complexity number (CCN). Counts
//!   decision-point nodes inside each function body.
//! - **function-length** — line count of the function body.
//! - **parameter-count** — number of declared parameters.
//!
//! Languages bundled: Rust, TypeScript, TSX, JavaScript, Python, Go,
//! Java, C, C++, Clojure, C#. Each language is declared as one cohesive
//! [`language::LanguageSupport`] in the registry (identity, extensions, LSP
//! suggestion, unit kinds) plus its `LanguageSpec` node tables in
//! [`spec`]. Adding a language is a single, well-isolated change — see
//! `.context/language-support.md`.
//!
//! Files in unsupported languages are silently skipped (the caller
//! sees no findings for them).

use std::collections::HashSet;
use std::path::Path;

use tree_sitter::Node;

mod ast;
pub mod language;
mod spec;
pub mod symbols;

pub use ast::{ast_query, parse, query, AstQueryError, QueryMatch};
pub use language::{
    for_language as language_support, language_for_path, mason_suggestion, registry,
    LanguageSupport, UnitKind,
};
pub use spec::{
    language_from_lsp_id, language_from_name, Language, LanguageSpec, VisibilityStrategy,
};
pub use symbols::{extract, extract_with_language, Definition, SymbolKind};
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
    /// The function's own name.
    pub name: String,
    /// Its symbol id within the file (`Parser::parse`, `main~2`) — the
    /// same id [`extract`] gives it.
    pub id: String,
    /// Its symbol kind: function, method or constructor.
    pub kind: SymbolKind,
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
    /// The names of what it's inside, outermost first — its symbol's
    /// container (empty at top level).
    pub container_path: Vec<String>,
    /// Coarse public/private classification (heuristic per language).
    pub visibility: Visibility,
    /// Whether the function carries a doc comment (or, for Python/Clojure, a
    /// docstring). Per-language detection — see [`LanguageSpec::doc`]. The
    /// `oxplow.doc_coverage` metric is documented-public ÷ public over this.
    pub has_doc: bool,
}

/// A code "unit" — the generic, language-agnostic structural element the
/// language-support epic (tsk320) builds navigation / roll-ups on. One per
/// function, class-like container, module/namespace, or package the file
/// participates in. The set of kinds a language exposes is declared on its
/// [`language::LanguageSupport::unit_kinds`].
#[derive(Debug, Clone, PartialEq)]
pub struct CodeUnit {
    /// What kind of unit this is.
    pub kind: UnitKind,
    /// The unit's name (e.g. function/class name, package directory).
    pub name: String,
    /// Outer-to-inner names of the named-declaration ancestors this unit
    /// lives inside. Empty for top-level units.
    pub container_path: Vec<String>,
    /// 1-based start line; `0` for synthetic units (e.g. the package, which
    /// is path-derived rather than a single AST node).
    pub start_line: u32,
    /// 1-based end line; `0` for synthetic units.
    pub end_line: u32,
}

/// Analyze a single file. Returns an empty Vec for unsupported
/// languages (or files that fail to parse).
pub fn analyze_file(path: &str, source: &str) -> Vec<FunctionMetrics> {
    let Some(lang) = language_for_path(path) else {
        return Vec::new();
    };
    analyze_with_language(path, source, lang)
}

/// List the code units in a file — functions, class-like types,
/// modules/namespaces, and (when the language declares it) the package the
/// file belongs to: the navigable subset of its [`extract`]ed definitions,
/// deterministic and offline (tree-sitter). Returns an empty Vec for
/// unsupported / unparseable input.
///
/// Units are returned in a stable order: package (if any) → the rest in
/// source order.
pub fn list_units(path: &str, source: &str) -> Vec<CodeUnit> {
    let Some(lang) = language_for_path(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    // Package: path-derived, only when the language declares it as a unit
    // (Go, Java). Named by the file's parent directory.
    if language::for_language(lang)
        .unit_kinds
        .contains(&UnitKind::Package)
    {
        if let Some(pkg) = package_for_path(path) {
            out.push(CodeUnit {
                kind: UnitKind::Package,
                name: pkg,
                container_path: Vec::new(),
                start_line: 0,
                end_line: 0,
            });
        }
    }
    out.extend(
        extract_with_language(source, lang)
            .into_iter()
            .filter_map(|d| {
                Some(CodeUnit {
                    kind: unit_kind(d.kind)?,
                    name: d.name,
                    container_path: d.container,
                    start_line: d.start_line,
                    end_line: d.end_line,
                })
            }),
    );
    out
}

/// The package name for a path — its parent directory (repo-relative).
/// `None` for a top-level file (no parent component).
fn package_for_path(path: &str) -> Option<String> {
    let parent = std::path::Path::new(path).parent()?;
    let s = parent.to_str()?;
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// The unit a definition is, if it is one: callables are functions,
/// types are classes, modules and namespaces are modules; fields,
/// constants, variables, enum members and macros aren't units.
fn unit_kind(kind: SymbolKind) -> Option<UnitKind> {
    if kind.is_callable() {
        Some(UnitKind::Function)
    } else if kind.is_type() {
        Some(UnitKind::Class)
    } else if matches!(kind, SymbolKind::Module | SymbolKind::Namespace) {
        Some(UnitKind::Module)
    } else {
        None
    }
}

/// Like `analyze_file` but with the language explicitly chosen
/// (useful when the path doesn't reveal the language, e.g. content
/// fetched from a git ref into a temp buffer).
pub fn analyze_with_language(path: &str, source: &str, language: Language) -> Vec<FunctionMetrics> {
    let Some(tree) = parse(source, language) else {
        return Vec::new();
    };
    let spec = language.spec();
    let src = source.as_bytes();
    let callables: Vec<(Definition, Node<'_>)> = symbols::extract_tree(&tree, src, language)
        .into_iter()
        .filter(|(d, _)| d.kind.is_callable())
        .collect();
    let bodies: HashSet<usize> = callables.iter().map(|(_, n)| n.id()).collect();
    callables
        .into_iter()
        .map(|(d, node)| function_metrics(d, node, src, spec, path, &bodies))
        .collect()
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
    def: Definition,
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    path: &str,
    bodies: &HashSet<usize>,
) -> FunctionMetrics {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    FunctionMetrics {
        path: path.into(),
        complexity: count_decision_points(node, src, spec, bodies) + 1,
        length: end_line.saturating_sub(start_line) + 1,
        parameter_count: count_parameters(node, src, spec),
        start_line,
        end_line,
        visibility: visibility_for(node, src, spec, &def.name),
        has_doc: has_doc_for(node, src, spec),
        name: def.name,
        id: def.id,
        kind: def.kind,
        container_path: def.container,
    }
}

/// Whether `node` (a function) carries documentation, per the language's
/// [`DocStrategy`]. Used by the `oxplow.doc_coverage` metric.
fn has_doc_for(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> bool {
    use crate::spec::DocStrategy::*;
    match spec.doc {
        None => false,
        PrecedingComment(prefixes) => {
            // Climb out of export wrappers (TS/JS `export function …`): the doc
            // comment precedes the `export_statement`, not the inner declaration.
            let mut item = node;
            while let Some(parent) = item.parent() {
                if parent.kind().contains("export") {
                    item = parent;
                } else {
                    break;
                }
            }
            // Walk back over any attribute / decorator / annotation nodes that
            // sit between the item and its doc comment, to the nearest real
            // preceding sibling. A doc comment there — text starting with one of
            // the prefixes (`///` etc., NOT a plain `//`) — marks it documented.
            let mut prev = item.prev_named_sibling();
            while let Some(p) = prev {
                let k = p.kind();
                if k.contains("attribute") || k.contains("annotation") || k.contains("decorator") {
                    prev = p.prev_named_sibling();
                    continue;
                }
                if k.contains("comment") {
                    if let Ok(text) = p.utf8_text(src) {
                        let t = text.trim_start();
                        return prefixes.iter().any(|pre| t.starts_with(pre));
                    }
                }
                break;
            }
            false
        }
        PythonDocstring => {
            // The function body's first statement is a bare string literal.
            let Some(body) = node.child_by_field_name("body") else {
                return false;
            };
            let Some(first) = body.named_child(0) else {
                return false;
            };
            match first.kind() {
                "expression_statement" => {
                    first.named_child(0).is_some_and(|s| s.kind() == "string")
                }
                "string" => true,
                _ => false,
            }
        }
        ClojureDocstring => {
            // A string literal in the docstring position — a direct child of the
            // `(defn …)` form appearing before the parameter vector.
            let mut idx = 0;
            while let Some(child) = node.named_child(idx) {
                match child.kind() {
                    "str_lit" => return true,
                    "vec_lit" => return false, // reached the params, no docstring
                    _ => {}
                }
                idx += 1;
            }
            false
        }
    }
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

fn count_decision_points(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    bodies: &HashSet<usize>,
) -> u32 {
    let mut count = 0u32;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, src, spec, bodies);
    }
    count
}

/// Decisions under `node`, up to (not into) a nested callable
/// definition — its decisions are its own. An anonymous closure isn't
/// one, so its decisions count toward the function around it.
fn count_decision_subtree(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    bodies: &HashSet<usize>,
) -> u32 {
    if bodies.contains(&node.id()) {
        return 0;
    }
    let mut count = u32::from(is_decision_node(node, src, spec));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, src, spec, bodies);
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

/// How many leading lines of a file count as its "header" for the
/// generated-file check. Codegen tools put their marker in the first few
/// lines; scanning further risks flagging hand-written files that merely
/// *mention* the phrases (in a string, a late comment, …).
const GENERATED_HEADER_LINES: usize = 10;

/// Heuristic: is this file codegen output rather than hand-written source?
/// Generated artifacts (tauri-specta bindings, protobuf stubs, …) shouldn't
/// count toward code-shape metrics — a 3k-line bindings file read as one
/// giant "function" dominates every tail metric (tsk68).
///
/// Two signals, either suffices:
/// - **path**: a `generated` path segment, or a `.generated.` /
///   `_generated.` basename infix — the conventional codegen locations.
/// - **header**: a do-not-edit-style marker (`@generated`, `do not edit`,
///   `autogenerated`, `automatically generated`) in the first few lines,
///   case-insensitive. Only the header counts — see
///   [`GENERATED_HEADER_LINES`].
pub fn is_generated_source(path: impl AsRef<Path>, text: &str) -> bool {
    let p = path.as_ref();
    if p.components().any(|c| c.as_os_str() == "generated") {
        return true;
    }
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        if name.contains(".generated.") || name.contains("_generated.") {
            return true;
        }
    }
    text.lines()
        .take(GENERATED_HEADER_LINES)
        .map(|l| l.to_ascii_lowercase())
        .any(|l| {
            l.contains("@generated")
                || l.contains("do not edit")
                || l.contains("autogenerated")
                || l.contains("automatically generated")
        })
}

#[cfg(test)]
mod tests;
