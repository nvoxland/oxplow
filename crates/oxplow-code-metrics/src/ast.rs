//! Generic, query-based AST access — the engine behind the metric substrate's
//! `ast_query` host capability (epic tsk213, P3). The per-function metric
//! walkers in [`crate`] are *one* consumer of tree-sitter; this module exposes
//! the grammars generically so a bundled or user-authored gauge plugin can run
//! an arbitrary tree-sitter S-expression query against source text and get back
//! a flat list of matches (no Starlark recursion needed).

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

use crate::spec::{language_from_name, Language};

/// One capture from a tree-sitter query: the capture name (`@foo`), the matched
/// source text, and its 0-based start/end position. Flat by design so a script
/// never has to walk a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryMatch {
    pub capture: String,
    pub text: String,
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
}

/// Why an [`ast_query`] / [`query`] call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstQueryError {
    /// The language name wasn't recognized (see [`language_from_name`]).
    UnknownLanguage(String),
    /// tree-sitter couldn't build a parse tree for the text.
    Parse,
    /// The S-expression query didn't compile against the grammar.
    BadQuery(String),
}

impl std::fmt::Display for AstQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AstQueryError::UnknownLanguage(l) => write!(f, "unknown language \"{l}\""),
            AstQueryError::Parse => write!(f, "could not parse source"),
            AstQueryError::BadQuery(e) => write!(f, "bad tree-sitter query: {e}"),
        }
    }
}

impl std::error::Error for AstQueryError {}

/// Parse `text` as `language` into a tree-sitter [`Tree`]. Returns `None` if the
/// grammar can't be loaded or the text can't be parsed. (The parser is built
/// fresh per call — same as the metric walker — so this is `Send`-friendly and
/// holds no shared state.)
pub fn parse(text: &str, language: Language) -> Option<Tree> {
    let spec = language.spec();
    let mut parser = Parser::new();
    if parser.set_language(&spec.tree_sitter_language()).is_err() {
        return None;
    }
    parser.parse(text, None)
}

/// Run a tree-sitter S-expression `sexpr` query over an already-parsed `tree`,
/// returning every capture as a flat [`QueryMatch`]. `src` must be the bytes the
/// tree was parsed from (used to slice the matched text).
pub fn query(
    tree: &Tree,
    src: &[u8],
    sexpr: &str,
    language: Language,
) -> Result<Vec<QueryMatch>, AstQueryError> {
    let ts_lang = language.tree_sitter_language();
    let q = Query::new(&ts_lang, sexpr).map_err(|e| AstQueryError::BadQuery(e.to_string()))?;
    let names = q.capture_names();
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(&q, tree.root_node(), src);
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let node = cap.node;
            let name = names.get(cap.index as usize).copied().unwrap_or("");
            let start = node.start_position();
            let end = node.end_position();
            out.push(QueryMatch {
                capture: name.to_string(),
                text: node.utf8_text(src).unwrap_or_default().to_string(),
                start_row: start.row,
                start_col: start.column,
                end_row: end.row,
                end_col: end.column,
            });
        }
    }
    Ok(out)
}

/// Parse `text` as `language` (by name, e.g. `"rust"`, `"typescript"`) and run a
/// tree-sitter `sexpr` query — the one-call entry the `ast_query` host
/// capability is built on.
pub fn ast_query(
    text: &str,
    language: &str,
    sexpr: &str,
) -> Result<Vec<QueryMatch>, AstQueryError> {
    let lang = language_from_name(language)
        .ok_or_else(|| AstQueryError::UnknownLanguage(language.to_string()))?;
    let tree = parse(text, lang).ok_or(AstQueryError::Parse)?;
    query(&tree, text.as_bytes(), sexpr, lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ast_query_matches_both_outer_and_inner_allow_attributes() {
        // `#[allow(...)]` is an `attribute_item`; `#![allow(...)]` is an
        // `inner_attribute_item`. A query naming only the former silently
        // undercounts crate/module-level suppressions — which are the ones that
        // matter MOST (an inner attribute mutes a lint for a whole file). Pin both
        // node names so the undercount can't come back (tsk44).
        let src = r#"
#![allow(clippy::unwrap_used)]

#[allow(dead_code)]
fn a() {}

#[derive(Debug)]
struct S;
"#;
        let q = "(attribute_item (attribute (identifier) @a)) \
                 (inner_attribute_item (attribute (identifier) @a))";
        let matches = ast_query(src, "rust", q).expect("query runs");
        let allows = matches.iter().filter(|m| m.text == "allow").count();
        assert_eq!(allows, 2, "one outer + one inner; `derive` is not an allow");
    }

    #[test]
    fn ast_query_counts_unsafe_blocks_in_rust() {
        let src = r#"
fn a() { unsafe { let x = 1; } }
fn b() { let y = 2; }
fn c() { unsafe { foo(); } }
"#;
        let matches = ast_query(src, "rust", "(unsafe_block) @u").expect("query runs");
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.capture == "u"));
        assert!(matches[0].text.starts_with("unsafe"));
    }

    #[test]
    fn ast_query_captures_text_and_position() {
        let src = "fn hello() {}\n";
        let matches =
            ast_query(src, "rust", "(function_item name: (identifier) @name)").expect("runs");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].capture, "name");
        assert_eq!(matches[0].text, "hello");
        assert_eq!(matches[0].start_row, 0);
        assert_eq!(matches[0].start_col, 3);
    }

    #[test]
    fn ast_query_rejects_unknown_language() {
        let err = ast_query("x", "cobol", "(x) @x").unwrap_err();
        assert_eq!(err, AstQueryError::UnknownLanguage("cobol".into()));
    }

    #[test]
    fn ast_query_rejects_malformed_query() {
        let err = ast_query("fn a() {}", "rust", "(nonsense_node) @x").unwrap_err();
        assert!(matches!(err, AstQueryError::BadQuery(_)));
    }

    #[test]
    fn ast_query_works_for_csharp() {
        // C# method declarations resolve through the new `Language::CSharp`
        // grammar (the catalog under oxplow-collect-plugin builds on this).
        let src = "class C { public void M(int x) { if (x > 0) { Do(); } } }";
        let methods = ast_query(src, "csharp", "(method_declaration) @m").expect("runs");
        assert_eq!(methods.len(), 1);
    }

    #[test]
    fn ast_query_works_across_languages() {
        // TypeScript: count `any` type annotations (a `predefined_type` node).
        let ts = "function f(x: any): any { return x; }";
        let anys = ast_query(ts, "typescript", "(predefined_type) @a").expect("runs");
        assert_eq!(anys.len(), 2);
        assert!(anys.iter().all(|m| m.text == "any"));
    }
}
