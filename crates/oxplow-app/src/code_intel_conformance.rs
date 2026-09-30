//! The code-intelligence conformance suite (P5.C6, `.context/lsp.md`):
//! what a [`CodeIntelligence`] provider must answer, as plain functions
//! over the trait and a [`CodeIntelProbe`] that names a covered file and a
//! symbol in it, and makes the provider report and lose a diagnostic. It
//! runs in-tree against the language servers (over the fake python
//! server) and, in P5.D, against an external provider.
//!
//! Each check is a [`Finding`] when it fails; an empty list passes.

use async_trait::async_trait;
use oxplow_domain::code_intel::{CallDirection, CodeIntelError, CodeIntelligence, Position, Range};
use oxplow_domain::StreamId;

pub use crate::work_items_conformance::Finding;

/// The symbol kinds a provider may name (the LSP's, by name).
const KINDS: &[&str] = &[
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum_member",
    "struct",
    "event",
    "operator",
    "type_parameter",
];

const SEVERITIES: &[&str] = &["error", "warning", "information", "hint"];

/// What the suite needs from the host.
#[async_trait]
pub trait CodeIntelProbe: Send + Sync {
    fn stream(&self) -> StreamId;
    /// A file the provider covers.
    fn file(&self) -> String;
    /// A position on a symbol in that file.
    fn symbol(&self) -> Position;
    /// Make the provider report a diagnostic with `message` for the file,
    /// and wait until it is recorded.
    async fn report(&self, message: &str);
    /// Make the provider go away (a crash), and wait until what it
    /// reported is gone.
    async fn lose(&self);
}

fn well_formed(r: &Range) -> bool {
    r.start.line >= 1 && r.start.col >= 1 && (r.start.line, r.start.col) <= (r.end.line, r.end.col)
}

/// Run every check against `provider`.
pub async fn suite(provider: &dyn CodeIntelligence, probe: &dyn CodeIntelProbe) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let (stream, file, at) = (probe.stream(), probe.file(), probe.symbol());

    // 1. Definition and references come back as 1-based locations.
    match provider.definition(&at).await {
        Ok(locs) if locs.is_empty() => fail("definition", "no location".into()),
        Ok(locs) if !locs.iter().all(|l| well_formed(&l.range)) => {
            fail("definition", format!("malformed ranges: {locs:?}"))
        }
        Ok(_) => {}
        Err(e) => fail("definition", e.to_string()),
    }
    match provider.references(&at, true).await {
        Ok(locs) if locs.is_empty() => fail("references", "no reference".into()),
        Ok(_) => {}
        Err(e) => fail("references", e.to_string()),
    }

    // 2. A hover says something.
    match provider.hover(&at).await {
        Ok(Some(h)) if !h.contents.trim().is_empty() => {}
        other => fail("hover", format!("no hover: {other:?}")),
    }

    // 3. Document symbols name known kinds, nesting through containers.
    match provider.document_symbols(stream, &file).await {
        Ok(symbols) if symbols.is_empty() => fail("symbols", format!("none in `{file}`")),
        Ok(symbols) => {
            if let Some(s) = symbols.iter().find(|s| !KINDS.contains(&s.kind.as_str())) {
                fail("symbols", format!("unknown kind `{}`", s.kind));
            }
            let paths: Vec<String> = symbols
                .iter()
                .map(|s| match &s.container {
                    Some(c) => format!("{c}::{}", s.name),
                    None => s.name.clone(),
                })
                .collect();
            if let Some(s) = symbols
                .iter()
                .find(|s| s.container.as_ref().is_some_and(|c| !paths.contains(c)))
            {
                fail(
                    "symbols",
                    format!(
                        "`{}`'s container `{:?}` isn't a symbol",
                        s.name, s.container
                    ),
                );
            }
        }
        Err(e) => fail("symbols", e.to_string()),
    }

    // 4. The call hierarchy answers (possibly empty).
    if let Err(e) = provider.call_hierarchy(&at, CallDirection::Incoming).await {
        fail("call_hierarchy", e.to_string());
    }

    // 5. A rename proposes edits carrying the new name, applying none.
    match provider.rename(&at, "renamed_by_conformance").await {
        Ok(edit) if edit.files.is_empty() => fail("rename", "no edits".into()),
        Ok(edit)
            if !edit
                .files
                .iter()
                .flat_map(|f| &f.edits)
                .all(|e| e.new_text == "renamed_by_conformance" && well_formed(&e.range)) =>
        {
            fail("rename", format!("edits don't carry the name: {edit:?}"))
        }
        Ok(_) => {}
        Err(e) => fail("rename", e.to_string()),
    }

    // 6. What the provider reports is read back; losing it clears it, and
    //    a lost provider says it isn't running — never an empty "clean".
    probe.report("reported by conformance").await;
    match provider.diagnostics(stream, &file).await {
        Ok(ds)
            if ds.iter().any(|d| {
                d.message == "reported by conformance"
                    && SEVERITIES.contains(&d.severity.as_str())
                    && well_formed(&d.range)
            }) => {}
        other => fail(
            "diagnostics",
            format!("the report isn't read back: {other:?}"),
        ),
    }
    probe.lose().await;
    match provider.diagnostics(stream, &file).await {
        Err(CodeIntelError::NotRunning(_)) => {}
        other => fail(
            "diagnostics",
            format!("a lost provider doesn't say it isn't running: {other:?}"),
        ),
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    struct AppProbe {
        svc: Arc<crate::Services>,
        stream: StreamId,
        dir: std::path::PathBuf,
    }

    impl AppProbe {
        async fn until(&self, want: impl Fn(usize) -> bool) {
            for _ in 0..200 {
                let n = self
                    .svc
                    .diagnostic_store
                    .list_file(self.stream.value(), "w.py".into())
                    .await
                    .unwrap()
                    .len();
                if want(n) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }

        async fn notify(&self, method: &str, params: serde_json::Value) {
            self.svc
                .lsp_sessions
                .notify_session(
                    &self.stream.to_string(),
                    "python",
                    self.dir.clone(),
                    method,
                    params,
                )
                .await
                .unwrap();
        }
    }

    #[async_trait]
    impl CodeIntelProbe for AppProbe {
        fn stream(&self) -> StreamId {
            self.stream
        }

        fn file(&self) -> String {
            "w.py".into()
        }

        fn symbol(&self) -> Position {
            Position {
                stream: self.stream,
                path: "w.py".into(),
                line: 3,
                col: 7,
            }
        }

        async fn report(&self, message: &str) {
            let uri = url::Url::from_file_path(self.dir.join("w.py"))
                .unwrap()
                .to_string();
            self.notify(
                "publish",
                serde_json::json!({ "uri": uri, "diagnostics": [
                    {"range": {"start": {"line": 2, "character": 6}, "end": {"line": 2, "character": 12}},
                     "severity": 2, "message": message}] }),
            )
            .await;
            self.until(|n| n > 0).await;
        }

        async fn lose(&self) {
            self.notify("die", serde_json::json!({})).await;
            self.until(|n| n == 0).await;
        }
    }

    /// The language servers (here the fake python one) are a conforming
    /// provider — publishes land in `v_diagnostic`, and a crash clears them.
    #[tokio::test]
    async fn the_language_servers_are_a_conforming_provider() {
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
        let stream = svc.streams.ensure_primary().await.unwrap().id;
        crate::lsp_diagnostics::spawn(svc.clone());
        let probe = AppProbe {
            svc: svc.clone(),
            stream,
            dir: std::path::PathBuf::from(
                &svc.streams.list_streams().await.unwrap()[0].worktree_path,
            ),
        };
        let findings = suite(&*svc.code_intel, &probe).await;
        assert_eq!(findings, vec![]);
    }
}
