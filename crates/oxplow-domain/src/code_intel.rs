//! The code-intelligence capability (P5.C5, `.context/lsp.md`): what a
//! language-aware provider answers about code — definitions, references,
//! hovers, symbols, call hierarchies, diagnostics, renames — as typed
//! values. The language servers are the built-in provider
//! (`oxplow_app::code_intel::LspProvider`). Positions and ranges are
//! 1-based (line and column), like `v_diagnostic` and editors; paths are
//! workspace-relative when inside the stream's workspace.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::StreamId;

/// A place in a stream's file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    #[schemars(with = "String")]
    pub stream: StreamId,
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub col: u32,
}

/// A 1-based point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Range {
    pub start: Point,
    pub end: Point,
}

/// A range in a file: workspace-relative, or absolute when outside the
/// workspace (a dependency's source).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    pub path: String,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hover {
    /// Markdown.
    pub contents: String,
    pub range: Option<Range>,
}

/// A declared symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Symbol {
    pub name: String,
    /// `function`, `class`, `method`, `module`, … (the LSP symbol kinds,
    /// by name).
    pub kind: String,
    /// The enclosing symbol's name path (`Widget`), when nested.
    pub container: Option<String>,
    pub location: Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallDirection {
    /// Who calls the symbol.
    Incoming,
    /// What the symbol calls.
    Outgoing,
}

/// One caller (incoming) or callee (outgoing) of a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Call {
    pub symbol: Symbol,
    /// Where the call happens (in the caller for incoming, in the symbol
    /// for outgoing).
    pub at: Vec<Range>,
}

/// A diagnostic a provider reported for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    /// `error`, `warning`, `information` or `hint`.
    pub severity: String,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    pub range: Range,
}

/// Text edits a rename would make, per file. Not applied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceEdit {
    pub files: Vec<FileEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileEdit {
    pub path: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeIntelError {
    /// No provider covers the file's language; the message says how to
    /// add one.
    #[error("{0}")]
    NoProvider(String),
    /// A provider covers the file's language but isn't running, so what
    /// it would report is unknown (not "nothing").
    #[error("{0}")]
    NotRunning(String),
    #[error("{0}")]
    Failed(String),
}

/// A source of code intelligence.
#[async_trait]
pub trait CodeIntelligence: Send + Sync {
    async fn definition(&self, at: &Position) -> Result<Vec<Location>, CodeIntelError>;
    async fn references(
        &self,
        at: &Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>, CodeIntelError>;
    async fn hover(&self, at: &Position) -> Result<Option<Hover>, CodeIntelError>;
    /// Every symbol declared in a file, nested ones carrying their
    /// container.
    async fn document_symbols(
        &self,
        stream: StreamId,
        path: &str,
    ) -> Result<Vec<Symbol>, CodeIntelError>;
    /// Symbols matching `query` across the workspace, for `language`.
    async fn workspace_symbols(
        &self,
        stream: StreamId,
        language: &str,
        query: &str,
    ) -> Result<Vec<Symbol>, CodeIntelError>;
    async fn call_hierarchy(
        &self,
        at: &Position,
        direction: CallDirection,
    ) -> Result<Vec<Call>, CodeIntelError>;
    /// What the provider last reported for a file. `NotRunning` when no
    /// provider for its language is running: empty always means clean.
    async fn diagnostics(
        &self,
        stream: StreamId,
        path: &str,
    ) -> Result<Vec<Diagnostic>, CodeIntelError>;
    /// The edits renaming the symbol at `at` would make.
    async fn rename(&self, at: &Position, new_name: &str) -> Result<WorkspaceEdit, CodeIntelError>;
}
