//! [`LspProvider`]: code intelligence (`oxplow_domain::code_intel`) from the
//! language servers (P5.C5, `.context/lsp.md`). The one place that builds
//! `textDocument/*` requests and maps what comes back — the LSP's
//! variants (`Location` / `LocationLink`, `DocumentSymbol` /
//! `SymbolInformation`, `MarkupContent` / `MarkedString`, 0-based
//! positions, URIs) — to the capability's typed values (1-based,
//! workspace-relative paths). A file's language comes from the servers'
//! configured extensions. Diagnostics are what the servers published
//! (`lsp_diagnostic`), not a pull.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::code_intel::{
    Call, CallDirection, CodeIntelError, CodeIntelligence, Diagnostic, FileEdit, FileOperation,
    Hover, Location, Point, Position, Range, Symbol, TextEdit, WorkspaceEdit,
};
use oxplow_domain::StreamId;
use serde_json::{json, Value};

use crate::lsp_sessions::{LspSessionError, LspSessionManager};
use crate::worktrees::WorktreeRouter;

pub struct LspProvider {
    /// A clone shares the sessions.
    sessions: LspSessionManager,
    worktrees: Arc<WorktreeRouter>,
    diagnostics: oxplow_db::SqliteDiagnosticStore,
    /// How long a request may wait for its server's answer.
    request_timeout: std::time::Duration,
}

impl LspProvider {
    pub fn new(
        sessions: LspSessionManager,
        worktrees: Arc<WorktreeRouter>,
        diagnostics: oxplow_db::SqliteDiagnosticStore,
        request_timeout: std::time::Duration,
    ) -> Self {
        Self {
            sessions,
            worktrees,
            diagnostics,
            request_timeout,
        }
    }

    async fn workspace(&self, stream: StreamId) -> Result<PathBuf, CodeIntelError> {
        self.worktrees
            .resolve_strict(Some(&stream.to_string()))
            .await
            .map_err(|e| CodeIntelError::Failed(e.to_string()))
    }

    /// The language whose configured server covers `path`. When none
    /// does, the error names the file's language (by extension) and the
    /// server to install, like a missing server's.
    fn language_of(&self, path: &str) -> Result<String, CodeIntelError> {
        self.sessions.language_for_path(path).ok_or_else(|| {
            match oxplow_code_metrics::plugin::language_for_path(path) {
                Some(language) => CodeIntelError::NoProvider(
                    LspSessionError::NoConfig(language.name().to_string()).to_string(),
                ),
                None => CodeIntelError::NoProvider(format!(
                    "no language server covers `{path}` — add an lsp.servers entry (with its \
                     `extensions`) to .oxplow/project.yaml, or install one via \
                     the `lsp.install_server` command"
                )),
            }
        })
    }

    /// Send `method` to the server for `language` in `stream`'s workspace.
    async fn request(
        &self,
        stream: StreamId,
        language: &str,
        method: &str,
        params: Value,
    ) -> Result<(Value, PathBuf), CodeIntelError> {
        let ws = self.workspace(stream).await?;
        let proxy = self
            .sessions
            .ensure(&stream.to_string(), language, ws.clone())
            .await
            .map_err(|e| match e {
                e @ LspSessionError::NoConfig(_) => CodeIntelError::NoProvider(e.to_string()),
                e => CodeIntelError::Failed(e.to_string()),
            })?;
        let result = tokio::time::timeout(self.request_timeout, proxy.request(method, params))
            .await
            .map_err(|_| {
                CodeIntelError::Failed(format!(
                    "the {language} server didn't answer `{method}` in {}s",
                    self.request_timeout.as_secs_f32()
                ))
            })?
            .map_err(|e| CodeIntelError::Failed(e.to_string()))?;
        Ok((result, ws))
    }

    /// A request about the document at `at`, with its position.
    async fn at(
        &self,
        at: &Position,
        method: &str,
        extra: Value,
    ) -> Result<(Value, PathBuf), CodeIntelError> {
        let language = self.language_of(&at.path)?;
        let ws = self.workspace(at.stream).await?;
        let mut params = json!({
            "textDocument": { "uri": uri_of(&ws, &at.path) },
            "position": {
                "line": at.line.saturating_sub(1),
                "character": at.col.saturating_sub(1),
            },
        });
        if let (Value::Object(p), Value::Object(e)) = (&mut params, extra) {
            p.extend(e);
        }
        self.request(at.stream, &language, method, params).await
    }
}

/// The `file://` URI of `path` in `ws`.
fn uri_of(ws: &Path, path: &str) -> String {
    url::Url::from_file_path(ws.join(path))
        .map(|u| u.to_string())
        .unwrap_or_else(|()| format!("file://{}", ws.join(path).display()))
}

/// `uri` as a workspace-relative path, or absolute when outside `ws`.
fn path_of(ws: &Path, uri: &str) -> String {
    let Some(file) = url::Url::parse(uri)
        .ok()
        .and_then(|u| u.to_file_path().ok())
    else {
        return uri.to_string();
    };
    let canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
    file.strip_prefix(ws)
        .or_else(|_| file.strip_prefix(&canon))
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.to_string_lossy().into_owned())
}

fn point(v: &Value) -> Option<Point> {
    Some(Point {
        line: v.get("line")?.as_u64()? as u32 + 1,
        col: v.get("character")?.as_u64()? as u32 + 1,
    })
}

fn range(v: &Value) -> Option<Range> {
    Some(Range {
        start: point(v.get("start")?)?,
        end: point(v.get("end")?)?,
    })
}

/// A `Location` or `LocationLink`.
fn location(ws: &Path, v: &Value) -> Option<Location> {
    let uri = v.get("uri").or_else(|| v.get("targetUri"))?.as_str()?;
    let r = v
        .get("range")
        .or_else(|| v.get("targetSelectionRange"))
        .or_else(|| v.get("targetRange"))?;
    Some(Location {
        path: path_of(ws, uri),
        range: range(r)?,
    })
}

/// `Location | Location[] | LocationLink[] | null`.
fn locations(ws: &Path, v: &Value) -> Vec<Location> {
    match v {
        Value::Array(items) => items.iter().filter_map(|i| location(ws, i)).collect(),
        Value::Null => Vec::new(),
        one => location(ws, one).into_iter().collect(),
    }
}

/// The LSP `SymbolKind` by name.
fn symbol_kind(n: u64) -> &'static str {
    match n {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum_member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        _ => "type_parameter",
    }
}

/// A `DocumentSymbol` tree (flattened, children carrying their container)
/// or `SymbolInformation` list (container from the server).
fn symbols(
    ws: &Path,
    path: Option<&str>,
    v: &Value,
    container: Option<&str>,
    out: &mut Vec<Symbol>,
) {
    let Some(items) = v.as_array() else {
        return;
    };
    for item in items {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let kind = symbol_kind(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
        // SymbolInformation / WorkspaceSymbol carry a location: the whole
        // symbol, which is also where it is.
        if let Some(loc) = item.get("location").and_then(|l| location(ws, l)) {
            out.push(Symbol {
                name: name.to_string(),
                kind: kind.to_string(),
                container: item
                    .get("containerName")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
                    .map(str::to_string),
                extent: loc.range,
                location: loc,
            });
            continue;
        }
        // DocumentSymbol: in `path`, nested through `children`; its name
        // is `selectionRange`, the whole of it `range`.
        let Some(path) = path else {
            continue;
        };
        let extent = item.get("range").and_then(range);
        let Some(name_at) = item.get("selectionRange").and_then(range).or(extent) else {
            continue;
        };
        out.push(Symbol {
            name: name.to_string(),
            kind: kind.to_string(),
            container: container.map(str::to_string),
            location: Location {
                path: path.to_string(),
                range: name_at,
            },
            extent: extent.unwrap_or(name_at),
        });
        if let Some(children) = item.get("children") {
            let nested = match container {
                Some(c) => format!("{c}::{name}"),
                None => name.to_string(),
            };
            symbols(ws, Some(path), children, Some(&nested), out);
        }
    }
}

/// `MarkupContent | MarkedString | MarkedString[]` as markdown.
fn hover_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(hover_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(o) => match (o.get("value"), o.get("language")) {
            (Some(Value::String(code)), Some(Value::String(lang))) => {
                format!("```{lang}\n{code}\n```")
            }
            (Some(Value::String(text)), _) => text.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// A `CallHierarchyItem` as a symbol.
fn call_item(ws: &Path, v: &Value) -> Option<Symbol> {
    let extent = v.get("range").and_then(range);
    let name_at = v.get("selectionRange").and_then(range).or(extent)?;
    Some(Symbol {
        name: v.get("name")?.as_str()?.to_string(),
        kind: symbol_kind(v.get("kind").and_then(Value::as_u64).unwrap_or(0)).to_string(),
        container: v.get("detail").and_then(Value::as_str).map(str::to_string),
        location: Location {
            path: path_of(ws, v.get("uri")?.as_str()?),
            range: name_at,
        },
        extent: extent.unwrap_or(name_at),
    })
}

#[async_trait]
impl CodeIntelligence for LspProvider {
    async fn definition(&self, at: &Position) -> Result<Vec<Location>, CodeIntelError> {
        let (v, ws) = self.at(at, "textDocument/definition", json!({})).await?;
        Ok(locations(&ws, &v))
    }

    async fn references(
        &self,
        at: &Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>, CodeIntelError> {
        let (v, ws) = self
            .at(
                at,
                "textDocument/references",
                json!({ "context": { "includeDeclaration": include_declaration } }),
            )
            .await?;
        Ok(locations(&ws, &v))
    }

    async fn hover(&self, at: &Position) -> Result<Option<Hover>, CodeIntelError> {
        let (v, _) = self.at(at, "textDocument/hover", json!({})).await?;
        if v.is_null() {
            return Ok(None);
        }
        let contents = hover_text(v.get("contents").unwrap_or(&Value::Null));
        Ok((!contents.is_empty()).then(|| Hover {
            contents,
            range: v.get("range").and_then(range),
        }))
    }

    async fn document_symbols(
        &self,
        stream: StreamId,
        path: &str,
    ) -> Result<Vec<Symbol>, CodeIntelError> {
        let language = self.language_of(path)?;
        let ws = self.workspace(stream).await?;
        let (v, ws) = self
            .request(
                stream,
                &language,
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": uri_of(&ws, path) } }),
            )
            .await?;
        let mut out = Vec::new();
        symbols(&ws, Some(path), &v, None, &mut out);
        Ok(out)
    }

    async fn workspace_symbols(
        &self,
        stream: StreamId,
        language: &str,
        query: &str,
    ) -> Result<Vec<Symbol>, CodeIntelError> {
        let (v, ws) = self
            .request(
                stream,
                language,
                "workspace/symbol",
                json!({ "query": query }),
            )
            .await?;
        let mut out = Vec::new();
        symbols(&ws, None, &v, None, &mut out);
        Ok(out)
    }

    async fn call_hierarchy(
        &self,
        at: &Position,
        direction: CallDirection,
    ) -> Result<Vec<Call>, CodeIntelError> {
        let (prepared, ws) = self
            .at(at, "textDocument/prepareCallHierarchy", json!({}))
            .await?;
        let Some(item) = prepared.as_array().and_then(|a| a.first()).cloned() else {
            return Ok(Vec::new());
        };
        let (method, side) = match direction {
            CallDirection::Incoming => ("callHierarchy/incomingCalls", "from"),
            CallDirection::Outgoing => ("callHierarchy/outgoingCalls", "to"),
        };
        let language = self.language_of(&at.path)?;
        let (calls, _) = self
            .request(at.stream, &language, method, json!({ "item": item }))
            .await?;
        Ok(calls
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| {
                        Some(Call {
                            symbol: call_item(&ws, c.get(side)?)?,
                            at: c
                                .get("fromRanges")
                                .and_then(Value::as_array)
                                .map(|rs| rs.iter().filter_map(range).collect())
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn diagnostics(
        &self,
        stream: StreamId,
        path: &str,
    ) -> Result<Vec<Diagnostic>, CodeIntelError> {
        // Only a running server's reports mean anything: with none, the
        // file's diagnostics are unknown, and an empty list would read as
        // clean.
        let language = self.language_of(path)?;
        if !self
            .sessions
            .is_running(&stream.to_string(), &language)
            .await
        {
            return Err(CodeIntelError::NotRunning(format!(
                "the {language} server isn't running in this stream, so `{path}`'s \
                 diagnostics are unknown — any other code_* request on a {language} file \
                 starts it"
            )));
        }
        let rows = self
            .diagnostics
            .list_file(stream.value(), path.to_string())
            .await
            .map_err(|e| CodeIntelError::Failed(e.to_string()))?;
        let p = |line: i64, col: i64| Point {
            line: line as u32,
            col: col as u32,
        };
        Ok(rows
            .into_iter()
            .map(|r| Diagnostic {
                severity: r.severity,
                message: r.message,
                source: r.source,
                code: r.code,
                range: Range {
                    start: p(r.line, r.col),
                    end: p(r.end_line, r.end_col),
                },
            })
            .collect())
    }

    async fn rename(&self, at: &Position, new_name: &str) -> Result<WorkspaceEdit, CodeIntelError> {
        let (v, ws) = self
            .at(at, "textDocument/rename", json!({ "newName": new_name }))
            .await?;
        Ok(workspace_edit(&ws, &v))
    }
}

/// An LSP `WorkspaceEdit` as ours: `documentChanges` when the server
/// sends it (text edits and file operations, in order), else `changes`.
fn workspace_edit(ws: &Path, v: &Value) -> WorkspaceEdit {
    let text_edits = |edits: &Value| -> Vec<TextEdit> {
        edits
            .as_array()
            .map(|es| {
                es.iter()
                    .filter_map(|e| {
                        Some(TextEdit {
                            range: range(e.get("range")?)?,
                            new_text: e.get("newText")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let uri = |c: &Value, key: &str| c.get(key).and_then(Value::as_str).map(|u| path_of(ws, u));
    let mut out = WorkspaceEdit::default();
    if let Some(doc_changes) = v.get("documentChanges").and_then(Value::as_array) {
        for change in doc_changes {
            let op = match change.get("kind").and_then(Value::as_str) {
                Some("create") => uri(change, "uri").map(|path| FileOperation::Create { path }),
                Some("delete") => uri(change, "uri").map(|path| FileOperation::Delete { path }),
                Some("rename") => uri(change, "oldUri")
                    .zip(uri(change, "newUri"))
                    .map(|(from, to)| FileOperation::Rename { from, to }),
                _ => {
                    if let Some(path) = change.get("textDocument").and_then(|d| uri(d, "uri")) {
                        out.files.push(FileEdit {
                            path,
                            edits: text_edits(change.get("edits").unwrap_or(&Value::Null)),
                        });
                    }
                    None
                }
            };
            out.operations.extend(op);
        }
    } else if let Some(changes) = v.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            out.files.push(FileEdit {
                path: path_of(ws, uri),
                edits: text_edits(edits),
            });
        }
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Services over a repo whose python files the fake server covers.
    async fn fixture() -> (Arc<crate::Services>, tempfile::TempDir, StreamId) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        std::fs::write(
            dir.path().join("w.py"),
            "\n\nclass Widget:\n    def spin(self): pass\n",
        )
        .unwrap();
        let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        svc.config
            .write()
            .unwrap()
            .lsp_servers
            .push(crate::lsp_fake::config("python", &["py"]));
        let stream = svc.streams.ensure_primary().await.unwrap();
        (svc, dir, stream.id)
    }

    fn at(stream: StreamId, line: u32, col: u32) -> Position {
        Position {
            stream,
            path: "w.py".into(),
            line,
            col,
        }
    }

    fn r(l1: u32, c1: u32, l2: u32, c2: u32) -> Range {
        Range {
            start: Point { line: l1, col: c1 },
            end: Point { line: l2, col: c2 },
        }
    }

    /// P5.C5: the language server's answers come back typed — 1-based
    /// ranges, workspace-relative paths.
    #[tokio::test]
    async fn the_servers_answers_come_back_typed() {
        let (svc, _dir, stream) = fixture().await;
        let ci = &svc.code_intel;
        assert_eq!(
            ci.definition(&at(stream, 3, 7)).await.unwrap(),
            vec![Location {
                path: "w.py".into(),
                range: r(3, 5, 3, 10)
            }]
        );
        assert_eq!(
            ci.references(&at(stream, 3, 7), true).await.unwrap().len(),
            2
        );
        assert_eq!(
            ci.hover(&at(stream, 3, 7)).await.unwrap(),
            Some(Hover {
                contents: "**class** Widget".into(),
                range: Some(r(3, 5, 3, 10))
            })
        );
        let symbols = ci.document_symbols(stream, "w.py").await.unwrap();
        let names: Vec<(&str, &str, Option<&str>)> = symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str(), s.container.as_deref()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Widget", "class", None),
                ("spin", "method", Some("Widget"))
            ]
        );
        let found = ci.workspace_symbols(stream, "python", "Wid").await.unwrap();
        assert_eq!(found[0].location.path, "w.py");
        let callers = ci
            .call_hierarchy(&at(stream, 4, 9), CallDirection::Incoming)
            .await
            .unwrap();
        assert_eq!(callers[0].symbol.name, "main");
        assert_eq!(callers[0].at, vec![r(10, 5, 10, 9)]);
        let edit = ci.rename(&at(stream, 3, 7), "Gadget").await.unwrap();
        assert_eq!(
            edit.files,
            vec![FileEdit {
                path: "w.py".into(),
                edits: vec![TextEdit {
                    range: r(3, 7, 3, 13),
                    new_text: "Gadget".into()
                }]
            }]
        );
    }

    /// tsk571: a rename answer's `documentChanges` wins over `changes`,
    /// and its file operations come through in order.
    #[test]
    fn a_rename_prefers_document_changes_and_keeps_file_operations() {
        let ws = Path::new("/repo");
        let edit = |l: u32| json!({ "range": { "start": { "line": l, "character": 0 }, "end": { "line": l, "character": 3 } }, "newText": "New" });
        let v = json!({
            "changes": { "file:///repo/stale.py": [edit(0)] },
            "documentChanges": [
                { "kind": "rename", "oldUri": "file:///repo/old.py", "newUri": "file:///repo/new.py" },
                { "textDocument": { "uri": "file:///repo/new.py", "version": 1 }, "edits": [edit(2)] },
                { "kind": "create", "uri": "file:///repo/made.py" },
                { "kind": "delete", "uri": "file:///repo/gone.py" }
            ]
        });
        let got = workspace_edit(ws, &v);
        assert_eq!(
            got.files,
            vec![FileEdit {
                path: "new.py".into(),
                edits: vec![TextEdit {
                    range: r(3, 1, 3, 4),
                    new_text: "New".into()
                }]
            }]
        );
        assert_eq!(
            got.operations,
            vec![
                FileOperation::Rename {
                    from: "old.py".into(),
                    to: "new.py".into()
                },
                FileOperation::Create {
                    path: "made.py".into()
                },
                FileOperation::Delete {
                    path: "gone.py".into()
                },
            ]
        );
        // Without documentChanges, changes are the answer.
        let plain = workspace_edit(
            ws,
            &json!({ "changes": { "file:///repo/a.py": [edit(0)] } }),
        );
        assert_eq!(plain.files[0].path, "a.py");
        assert!(plain.operations.is_empty());
    }

    /// tsk571: a document symbol's location is its name; its extent is
    /// the whole symbol.
    #[test]
    fn a_symbol_has_its_name_and_its_extent() {
        let pos = |l: u32, c: u32| json!({ "line": l, "character": c });
        let v = json!([{
            "name": "spin", "kind": 6,
            "range": { "start": pos(3, 4), "end": pos(9, 0) },
            "selectionRange": { "start": pos(4, 8), "end": pos(4, 12) }
        }]);
        let mut out = Vec::new();
        symbols(Path::new("/repo"), Some("w.py"), &v, None, &mut out);
        assert_eq!(out[0].location.range, r(5, 9, 5, 13));
        assert_eq!(out[0].extent, r(4, 5, 10, 1));
    }

    /// A file no configured server covers says which server to install.
    #[tokio::test]
    async fn an_uncovered_file_names_the_server_to_install() {
        let (svc, _dir, stream) = fixture().await;
        let err = svc
            .code_intel
            .definition(&Position {
                stream,
                path: "src/lib.rs".into(),
                line: 1,
                col: 1,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, CodeIntelError::NoProvider(_)));
        assert!(err.to_string().contains("rust-analyzer"), "{err}");
    }

    /// Diagnostics are what the running server published.
    #[tokio::test]
    async fn diagnostics_are_the_published_ones() {
        let (svc, dir, stream) = fixture().await;
        svc.lsp_sessions
            .ensure(&stream.to_string(), "python", dir.path().to_path_buf())
            .await
            .unwrap();
        svc.diagnostic_store
            .replace_file(
                stream.value(),
                "python".into(),
                "w.py".into(),
                vec![oxplow_db::DiagnosticRow {
                    severity: "error".into(),
                    message: "bad".into(),
                    line: 3,
                    col: 1,
                    end_line: 3,
                    end_col: 6,
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        let got = svc.code_intel.diagnostics(stream, "w.py").await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].range, r(3, 1, 3, 6));
    }

    /// With no server running, diagnostics are unknown, not clean: the
    /// answer says so instead of an empty list.
    #[tokio::test]
    async fn diagnostics_without_a_running_server_say_so() {
        let (svc, _dir, stream) = fixture().await;
        let err = svc
            .code_intel
            .diagnostics(stream, "w.py")
            .await
            .unwrap_err();
        assert!(matches!(err, CodeIntelError::NotRunning(_)), "{err:?}");
        assert!(err.to_string().contains("python"), "{err}");
        let err = svc
            .code_intel
            .diagnostics(stream, "src/lib.rs")
            .await
            .unwrap_err();
        assert!(matches!(err, CodeIntelError::NoProvider(_)), "{err:?}");
    }
}
