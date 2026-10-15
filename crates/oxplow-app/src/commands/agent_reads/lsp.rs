//! `lsp.read` operations: a stream's language servers asked for the
//! definition, references, hover, call hierarchy and symbols at a place
//! (`oxplow.code.*`). `code.symbols` falls back to oxplow's own offline
//! reading of the live file when no server serves it.

use oxplow_domain::code_intel::{CallDirection, CodeIntelError, Position};
use oxplow_domain::{CommandError, StreamId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, ServicesSlot};
use super::ids::{
    check_optional_stream, read_op, result_of, stream_id as checked_stream, stream_or_primary,
};

/// A place in a stream's file. Line and column are 1-based, like editors
/// and `v_diagnostic`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodePositionParams {
    pub stream_id: String,
    /// Workspace-relative path (`src/lib.rs`).
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub col: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeReferencesParams {
    pub stream_id: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// Include the declaration itself (default true).
    #[serde(default)]
    pub include_declaration: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeCallHierarchyParams {
    pub stream_id: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// `incoming` (who calls the symbol) or `outgoing` (what it calls).
    pub direction: CallDirection,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeSymbolsParams {
    /// The stream whose workspace to read; the caller's when absent.
    pub stream_id: Option<String>,
    /// Workspace-relative path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeWorkspaceSymbolsParams {
    /// The stream whose language server to ask; the caller's when absent.
    pub stream_id: Option<String>,
    /// The language server to ask (`rust`, `typescript`, `python`, …).
    pub language: String,
    /// Symbol name query (fuzzy, server-defined); empty lists what the
    /// server returns.
    pub query: String,
}

fn code_stream(field: &str, stream_id: &str) -> Result<StreamId, CommandError> {
    checked_stream("code", field, stream_id)
}

fn code_position(
    stream_id: &str,
    path: &str,
    line: u32,
    col: u32,
) -> Result<Position, CommandError> {
    if line == 0 || col == 0 {
        return Err(invalid("/line", "line and col are 1-based"));
    }
    Ok(Position {
        stream: code_stream("/stream_id", stream_id)?,
        path: path.to_string(),
        line,
        col,
    })
}

/// A missing or stopped server is the caller's to fix (install,
/// configure or start one); the message says how.
fn code_err(e: CodeIntelError) -> CommandError {
    match e {
        CodeIntelError::NoProvider(m) | CodeIntelError::NotRunning(m) => invalid("/stream_id", m),
        other => failed(other),
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        read_op(
            "lsp.read",
            "definition",
            services.clone(),
            |svc, _, p: CodePositionParams| async move {
                let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
                result_of(svc.code_intel.definition(&at).await.map_err(code_err)?)
            },
        ),
        read_op(
            "lsp.read",
            "references",
            services.clone(),
            |svc, _, p: CodeReferencesParams| async move {
                let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
                result_of(
                    svc.code_intel
                        .references(&at, p.include_declaration.unwrap_or(true))
                        .await
                        .map_err(code_err)?,
                )
            },
        ),
        read_op(
            "lsp.read",
            "hover",
            services.clone(),
            |svc, _, p: CodePositionParams| async move {
                let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
                result_of(svc.code_intel.hover(&at).await.map_err(code_err)?)
            },
        ),
        read_op(
            "lsp.read",
            "call_hierarchy",
            services.clone(),
            |svc, _, p: CodeCallHierarchyParams| async move {
                let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
                result_of(
                    svc.code_intel
                        .call_hierarchy(&at, p.direction)
                        .await
                        .map_err(code_err)?,
                )
            },
        ),
        read_op(
            "lsp.read",
            "symbols",
            services.clone(),
            |svc, actor, p: CodeSymbolsParams| async move {
                check_optional_stream("code_symbols", "/stream_id", p.stream_id.as_deref())?;
                let stream_id = stream_or_primary(&svc, &actor, p.stream_id).await?;
                let stream = code_stream("/stream_id", &stream_id)?;
                match svc.code_intel.document_symbols(stream, &p.path).await {
                    Ok(symbols) => {
                        result_of(json!({ "source": "language_server", "symbols": symbols }))
                    }
                    // No server for the language, or it isn't running: read
                    // the file as it is now.
                    Err(CodeIntelError::NoProvider(_) | CodeIntelError::NotRunning(_)) => {
                        let file = svc
                            .workspace_files
                            .read(Some(stream_id.as_str()), p.path.clone())
                            .await
                            .map_err(failed)?;
                        let units: Vec<Value> =
                            oxplow_code_metrics::list_units(&p.path, &file.content)
                                .iter()
                                .map(|u| {
                                    json!({
                                        "kind": u.kind.as_str(),
                                        "name": u.name,
                                        "containerPath": u.container_path,
                                        "startLine": u.start_line,
                                        "endLine": u.end_line,
                                    })
                                })
                                .collect();
                        result_of(json!({ "source": "tree_sitter", "symbols": units }))
                    }
                    Err(other) => Err(code_err(other)),
                }
            },
        ),
        read_op(
            "lsp.read",
            "workspace_symbols",
            services,
            |svc, actor, p: CodeWorkspaceSymbolsParams| async move {
                check_optional_stream(
                    "code_workspace_symbols",
                    "/stream_id",
                    p.stream_id.as_deref(),
                )?;
                let stream_id = stream_or_primary(&svc, &actor, p.stream_id).await?;
                let stream = code_stream("/stream_id", &stream_id)?;
                match svc
                    .code_intel
                    .workspace_symbols(stream, &p.language, &p.query)
                    .await
                {
                    Ok(symbols) => result_of(symbols),
                    Err(CodeIntelError::NoProvider(m) | CodeIntelError::NotRunning(m)) => {
                        Err(invalid(
                            "/language",
                            format!("{m} The stored symbols of captured files are in `v_symbol`."),
                        ))
                    }
                    Err(other) => Err(code_err(other)),
                }
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    fn anonymous() -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        }
    }

    async fn project() -> (tempfile::TempDir, std::sync::Arc<crate::Services>, String) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let stream = svc.streams.list_streams().await.unwrap()[0].id.to_string();
        (dir, svc, stream)
    }

    /// With no language server for a file, `code.symbols` reads the live
    /// file itself and says so; `code.workspace_symbols` points at the
    /// stored symbols; a position with no server says how to get one.
    #[tokio::test]
    async fn code_reads_fall_back_or_say_how_to_get_a_server() {
        let (dir, svc, stream) = project().await;
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/x.rs"),
            "fn alpha() {}\n\nstruct Beta;\n",
        )
        .unwrap();
        let symbols = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.code.symbols",
                json!({ "stream_id": stream, "path": "src/x.rs" }),
                false,
            )
            .await
            .unwrap()
            .result;
        assert_eq!(symbols["source"], "tree_sitter");
        assert_eq!(symbols["symbols"][0]["name"], "alpha");
        assert_eq!(symbols["symbols"][0]["startLine"], 1);

        let err = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.code.workspace_symbols",
                json!({ "stream_id": stream, "language": "rust", "query": "alpha" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("v_symbol"), "{err}");

        let err = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.code.hover",
                json!({ "stream_id": stream, "path": "src/x.rs", "line": 1, "col": 1 }),
                false,
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("rust-analyzer") && msg.contains("project.yaml"),
            "{msg}"
        );

        let err = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.code.definition",
                json!({ "stream_id": stream, "path": "src/x.rs", "line": 0, "col": 1 }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1-based"), "{err}");
    }
}
