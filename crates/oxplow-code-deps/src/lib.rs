//! Tree-sitter-based import / dependency extraction.
//!
//! For each supported language we walk the AST and emit one
//! [`ImportEdge`] per `use` / `import` / `require` / `#include` /
//! `(:require …)` declaration. The edge is purely *syntactic* — the
//! module string is the raw text the author wrote (e.g.
//! `"std::fs"`, `"@tauri-apps/api"`, `"./Foo"`, `"<stdio.h>"`).
//! Resolving that string to an in-repo file path is a separate
//! concern handled by callers that have the project's build-system
//! metadata (Cargo manifests, `tsconfig.json` paths, etc.).
//!
//! Languages supported (via `oxplow-code-metrics`'s grammar table):
//! Rust, TypeScript, TSX, JavaScript, Python, Go, Java, C, C++,
//! Clojure.
//!
//! Files in unsupported languages yield no edges (silently skipped).

use oxplow_code_metrics::{language_for_path, Language};
use serde::{Deserialize, Serialize};
use specta::Type;
use tree_sitter::{Node, Parser};

mod extractors;
mod zones;

pub use zones::{ZoneRules, ZonedImportEdge, ZONE_EXTERNAL, ZONE_OTHER};

#[cfg(test)]
mod tests;

/// What kind of dependency declaration produced this edge.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    /// Rust `use foo::bar;` / `pub use ...;` / `extern crate foo;`.
    Use,
    /// JS/TS `import { x } from "foo"` / `import * as x from "foo"`
    /// / `import "foo"`, plus `require("foo")` calls.
    Import,
    /// Python `import foo` / `from foo import bar`.
    PyImport,
    /// Go `import "foo"` (within a single-or-grouped import decl).
    GoImport,
    /// Java `import foo.bar.Baz;` / `import static foo.Bar.baz;`.
    JavaImport,
    /// C / C++ preprocessor `#include <stdio.h>` / `#include "foo.h"`.
    Include,
    /// C++ `using foo::bar;` / `using namespace foo;`.
    Using,
    /// Clojure `(ns my.ns (:require [foo.bar :as fb]))` or top-level
    /// `(require '[foo.bar :as fb])`.
    CljRequire,
}

/// One discovered dependency edge: "this file references this module
/// in this way at this span."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ImportEdge {
    /// Repo-relative path of the importing file (pass-through).
    pub from_path: String,
    /// The exact text of the import declaration as written.
    pub raw: String,
    /// Parsed module identifier — what the author imported. Format
    /// is language-native: `"std::fs"` for Rust, `"@scope/pkg"` or
    /// `"./relative"` for JS/TS, `"foo.bar"` for Python/Java,
    /// `"github.com/foo/bar"` for Go, `"<stdio.h>"` or `"foo.h"` for
    /// C/C++, `"foo.bar"` for Clojure.
    pub module: String,
    /// Declaration kind.
    pub kind: ImportKind,
    /// 1-based start line of the import declaration.
    pub start_line: u32,
    /// 1-based end line of the import declaration.
    pub end_line: u32,
}

/// Extract every import declaration from a source file.
///
/// Returns an empty Vec for unsupported languages or files that fail
/// to parse. Order is source order (top-to-bottom).
pub fn extract_imports(path: &str, source: &str) -> Vec<ImportEdge> {
    let Some(lang) = language_for_path(path) else {
        return Vec::new();
    };
    extract_with_language(path, source, lang)
}

/// Like [`extract_imports`] but with the language explicitly chosen
/// (useful when the path doesn't reveal it — e.g. content pulled
/// from a git ref into a temp buffer).
pub fn extract_with_language(path: &str, source: &str, language: Language) -> Vec<ImportEdge> {
    let mut parser = Parser::new();
    if parser
        .set_language(&language.tree_sitter_language())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let src = source.as_bytes();
    let mut out = Vec::new();
    extractors::walk(tree.root_node(), src, language, path, &mut out);
    out
}

/// Compute the diff between two sets of edges for the same file.
/// Returned tuple is `(added, removed)`. Order within each Vec is
/// stable on (kind, module).
pub fn diff_edges(
    before: &[ImportEdge],
    after: &[ImportEdge],
) -> (Vec<ImportEdge>, Vec<ImportEdge>) {
    let key = |e: &ImportEdge| (e.kind, e.module.clone());
    let before_set: std::collections::HashSet<_> = before.iter().map(key).collect();
    let after_set: std::collections::HashSet<_> = after.iter().map(key).collect();
    let mut added: Vec<ImportEdge> = after
        .iter()
        .filter(|e| !before_set.contains(&key(e)))
        .cloned()
        .collect();
    let mut removed: Vec<ImportEdge> = before
        .iter()
        .filter(|e| !after_set.contains(&key(e)))
        .cloned()
        .collect();
    added.sort_by_key(key);
    removed.sort_by_key(key);
    (added, removed)
}

/// Span helpers — kept module-private; extractors push edges with
/// their own line numbers.
pub(crate) fn lines(node: Node<'_>) -> (u32, u32) {
    (
        node.start_position().row as u32 + 1,
        node.end_position().row as u32 + 1,
    )
}
