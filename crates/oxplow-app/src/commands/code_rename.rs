//! `files.write/rename_symbol`, declared as `oxplow.code.rename { stream,
//! uri, line, col, new_name }`: rename the symbol at a place in a file
//! everywhere its language server finds it. The server proposes the edits
//! (`CodeIntelligence::rename`); this writes them into the stream's
//! worktree through its files (`WorkspaceFiles`), logs `file.saved` per
//! file changed, and — when the edit only changes text — undoes by
//! putting each file back as it was. Files the server creates, renames or
//! deletes are done after the text edits, in its order; such an edit has
//! no undo. See `.context/lsp.md`.

use std::sync::Arc;

use oxplow_domain::code_intel::{CodeIntelError, FileOperation, Position, TextEdit};
use oxplow_domain::events::schema::{FileSaved, FileSavedV1};
use oxplow_domain::{CommandCall, CommandError, Envelope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::file::{failed as file_failed, logged, stream_of};
use super::ops::Op;
use super::util::{invalid, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};

/// The command its operation is declared as.
pub const CODE_RENAME: &str = "oxplow.code.rename";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameSymbolInput {
    /// The stream whose worktree it's in (`str2`).
    pub stream: String,
    /// The file, by its URI (`oxplow:oxplow/file?path=src/lib.rs`).
    pub uri: String,
    /// 1-based, like editors and `v_diagnostic`.
    pub line: u32,
    /// 1-based, in UTF-16 code units (an LSP column).
    pub col: u32,
    /// What the symbol is called after.
    pub new_name: String,
}

/// `text` with `edits` made: each a 1-based range (columns in UTF-16
/// code units, as the servers count them) replaced by its text. Edits may
/// come in any order; two that overlap are refused.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> Result<String, String> {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let offset = |line: u32, col: u32| -> Result<usize, String> {
        let start = *line_starts
            .get(line.checked_sub(1).ok_or("lines count from 1")? as usize)
            .ok_or_else(|| format!("line {line} is past the end of the file"))?;
        let rest = &text[start..];
        let end_of_line = rest.find('\n').unwrap_or(rest.len());
        let mut units = col.checked_sub(1).ok_or("columns count from 1")? as usize;
        for (at, ch) in rest[..end_of_line].char_indices() {
            if units == 0 {
                return Ok(start + at);
            }
            units = units
                .checked_sub(ch.len_utf16())
                .ok_or_else(|| format!("column {col} of line {line} splits a character"))?;
        }
        if units == 0 {
            Ok(start + end_of_line)
        } else {
            Err(format!("column {col} is past the end of line {line}"))
        }
    };
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let from = offset(e.range.start.line, e.range.start.col)?;
            let to = offset(e.range.end.line, e.range.end.col)?;
            if to < from {
                return Err("an edit ends before it starts".to_string());
            }
            Ok((from, to, e.new_text.as_str()))
        })
        .collect::<Result<_, String>>()?;
    spans.sort_by_key(|(from, to, _)| (*from, *to));
    if spans.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err("two edits overlap".into());
    }
    let mut out = text.to_string();
    for (from, to, new_text) in spans.into_iter().rev() {
        out.replace_range(from..to, new_text);
    }
    Ok(out)
}

fn code_err(e: CodeIntelError) -> CommandError {
    match e {
        CodeIntelError::NoProvider(m) | CodeIntelError::NotRunning(m) => invalid("/uri", m),
        CodeIntelError::Failed(message) => CommandError::Failed { message },
    }
}

/// `files.write/rename_symbol`.
pub fn rename_symbol_op(services: ServicesSlot) -> Op {
    Op::new(
        "files.write",
        "rename_symbol",
        schema::<RenameSymbolInput>(),
        true,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let services = services.clone();
            Box::pin(async move {
                let input: RenameSymbolInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let (path, rev, _) = oxplow_domain::uri::build::file_of_uri(&input.uri)
                    .map_err(|e| invalid("/uri", e.to_string()))?;
                if rev.is_some() {
                    return Err(invalid(
                        "/uri",
                        "a rename changes the working tree: name the file without a revision",
                    ));
                }
                if input.new_name.trim().is_empty() {
                    return Err(invalid("/new_name", "say what the symbol is called after"));
                }
                let svc = services.get()?;
                let at = Position {
                    stream,
                    path,
                    line: input.line,
                    col: input.col,
                };
                let edit = svc
                    .code_intel
                    .rename(&at, &input.new_name)
                    .await
                    .map_err(code_err)?;
                if edit.files.is_empty() && edit.operations.is_empty() {
                    return Err(invalid(
                        "/line",
                        "the language server found nothing to rename there",
                    ));
                }
                let files = &svc.workspace_files;
                let stream_text = stream.to_string();
                // Every file is read and edited before any is written: an
                // edit that doesn't fit changes nothing.
                let mut written = Vec::new();
                for file in &edit.files {
                    let before = files
                        .read(Some(&stream_text), file.path.clone())
                        .await
                        .map_err(file_failed)?
                        .content;
                    let after =
                        apply_edits(&before, &file.edits).map_err(|e| CommandError::Failed {
                            message: format!(
                                "the server's edit of `{}` doesn't fit: {e}",
                                file.path
                            ),
                        })?;
                    written.push((file.path.clone(), before, after));
                }
                let mut events: Vec<Envelope> = Vec::new();
                for (path, _, after) in &written {
                    let bytes = after.len() as u64;
                    let saved = files
                        .write(Some(&stream_text), path.clone(), after.clone())
                        .await
                        .map_err(file_failed)?;
                    events.push(logged(
                        Envelope::typed::<FileSaved>(
                            invocation.actor().source(),
                            &FileSavedV1 {
                                stream: stream_text.clone(),
                                path: saved.path.clone(),
                                bytes,
                            },
                        ),
                        stream,
                        &saved.path,
                    ));
                }
                for op in &edit.operations {
                    match op {
                        FileOperation::Create { path } => {
                            files
                                .create_file(Some(&stream_text), path.clone(), String::new())
                                .await
                                .map_err(file_failed)?;
                        }
                        FileOperation::Rename { from, to } => {
                            files
                                .rename(Some(&stream_text), from.clone(), to.clone())
                                .await
                                .map_err(file_failed)?;
                        }
                        FileOperation::Delete { path } => {
                            files
                                .delete(Some(&stream_text), path.clone())
                                .await
                                .map_err(file_failed)?;
                        }
                    }
                }
                let inverse = edit
                    .operations
                    .is_empty()
                    .then(|| put_back(&stream_text, &written));
                Ok(HandlerOutput {
                    result: json!({
                        "files": written.iter().map(|(p, _, _)| p).collect::<Vec<_>>(),
                        "operations": edit.operations,
                        "undoable": inverse.is_some(),
                    }),
                    inverse,
                    events,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// The call that puts `written` files back as they were: one save, or a
/// sequence of them.
fn put_back(stream: &str, written: &[(String, String, String)]) -> CommandCall {
    let mut saves: Vec<CommandCall> = written
        .iter()
        .map(|(path, before, _)| CommandCall {
            name: "oxplow.file.save".into(),
            input: json!({ "stream": stream, "path": path, "content": before }),
        })
        .collect();
    if saves.len() == 1 {
        return saves.remove(0);
    }
    CommandCall {
        name: super::compose::SEQUENCE.into(),
        input: json!({
            "calls": saves
                .into_iter()
                .map(|c| json!({ "name": c.name, "input": c.input }))
                .collect::<Vec<_>>(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::code_intel::{Point, Range};

    fn edit(l1: u32, c1: u32, l2: u32, c2: u32, text: &str) -> TextEdit {
        TextEdit {
            range: Range {
                start: Point { line: l1, col: c1 },
                end: Point { line: l2, col: c2 },
            },
            new_text: text.into(),
        }
    }

    /// Edits land at their 1-based places, in any order, counting columns
    /// in UTF-16 units; ones that overlap or fall outside are refused.
    #[test]
    fn edits_land_at_their_places() {
        let text = "class Widget:\n    w = Widget()\n";
        let out = apply_edits(
            text,
            &[edit(2, 9, 2, 15, "Gadget"), edit(1, 7, 1, 13, "Gadget")],
        )
        .unwrap();
        assert_eq!(out, "class Gadget:\n    w = Gadget()\n");
        // "é" is one UTF-16 unit, "😀" two.
        let out = apply_edits("é😀x = 1\n", &[edit(1, 4, 1, 5, "y")]).unwrap();
        assert_eq!(out, "é😀y = 1\n");
        assert!(apply_edits(text, &[edit(1, 1, 1, 5, "a"), edit(1, 3, 1, 8, "b")]).is_err());
        assert!(apply_edits(text, &[edit(9, 1, 9, 2, "a")]).is_err());
        assert!(apply_edits(text, &[edit(1, 40, 1, 41, "a")]).is_err());
        // The end of a line is a place too.
        let out = apply_edits("ab\n", &[edit(1, 3, 1, 3, "c")]).unwrap();
        assert_eq!(out, "abc\n");
    }

    /// Over a running server: the rename is written into the worktree,
    /// logged, audited and undone by putting the file back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_rename_writes_the_servers_edits_and_undoes() {
        use oxplow_domain::Actor;
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let before = "\n\nclass Widget:\n    def spin(self): pass\n";
        std::fs::write(dir.path().join("w.py"), before).unwrap();
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        svc.config
            .write()
            .unwrap()
            .lsp_servers
            .push(crate::lsp_fake::config("python", &["py"]));
        crate::test_fixtures::activate(&svc).await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let out = svc
            .commands
            .run(
                &Actor::Human,
                CODE_RENAME,
                json!({
                    "stream": stream.id.to_string(),
                    "uri": oxplow_domain::uri::build::file_uri("w.py"),
                    "line": 3,
                    "col": 7,
                    "new_name": "Gadget",
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["files"], json!(["w.py"]));
        assert_eq!(out.result["undoable"], json!(true));
        let after = std::fs::read_to_string(dir.path().join("w.py")).unwrap();
        assert_eq!(after, "\n\nclass Gadget:\n    def spin(self): pass\n");
        svc.commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("w.py")).unwrap(),
            before
        );
        let err = svc
            .commands
            .run(
                &Actor::Human,
                CODE_RENAME,
                json!({
                    "stream": stream.id.to_string(),
                    "uri": "oxplow:oxplow/file?path=w.py&rev=git:HEAD",
                    "line": 3,
                    "col": 7,
                    "new_name": "Gadget",
                }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("without a revision"), "{err}");
    }
}
