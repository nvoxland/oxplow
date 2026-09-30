//! Persists what the language servers publish (`textDocument/publishDiagnostics`)
//! into `lsp_diagnostic`, so the semantic layer can read it as
//! `v_diagnostic`. Diagnostics are live state: the table is cleared at boot
//! and a server's rows are cleared when it (re)starts or stops. Debounced,
//! it emits [`OxplowEvent::DiagnosticsChanged`] so lenses re-run and logs
//! `code.diagnostics.changed@1` once per changed file with its counts
//! after the burst. See `.context/lsp.md`, `.context/semantic-layer.md`.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use oxplow_db::DiagnosticRow;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;

use crate::lsp_sessions::{LspSessionEvent, LspSessionStatus};
use crate::{OxplowEvent, Services};

/// How long a burst of publishes is coalesced before one event goes out.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// A parsed `publishDiagnostics`: the repo-relative path and its rows.
/// `None` when the URI isn't a file under `worktree`.
pub fn parse_publish(params: &Value, worktree: &Path) -> Option<(String, Vec<DiagnosticRow>)> {
    let uri = params.get("uri")?.as_str()?;
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let rel = path
        .strip_prefix(worktree)
        .map(Path::to_path_buf)
        .or_else(|_| {
            // macOS reports /private/var/... for /var/...; compare canonical forms.
            let canon = worktree.canonicalize().map_err(|_| ())?;
            path.strip_prefix(canon)
                .map(Path::to_path_buf)
                .map_err(|_| ())
        });
    let rel = rel.ok()?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    let rows = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|ds| ds.iter().filter_map(diagnostic_row).collect())
        .unwrap_or_default();
    Some((rel, rows))
}

fn diagnostic_row(d: &Value) -> Option<DiagnosticRow> {
    let range = d.get("range")?;
    let pos = |key: &str, field: &str| {
        range
            .get(key)
            .and_then(|p| p.get(field))
            .and_then(Value::as_i64)
            .map(|n| n + 1)
    };
    // LSP severity: 1 error, 2 warning, 3 information, 4 hint; absent is
    // up to the client, and editors treat it as an error.
    let severity = match d.get("severity").and_then(Value::as_i64) {
        Some(2) => "warning",
        Some(3) => "information",
        Some(4) => "hint",
        _ => "error",
    };
    let code = match d.get("code") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    Some(DiagnosticRow {
        severity: severity.into(),
        message: d.get("message")?.as_str()?.to_string(),
        source: d.get("source").and_then(Value::as_str).map(str::to_string),
        code,
        line: pos("start", "line")?,
        col: pos("start", "character")?,
        end_line: pos("end", "line")?,
        end_col: pos("end", "character")?,
    })
}

/// The stream (numeric id, worktree) an LSP session's `stream_id` names.
async fn stream_of(svc: &Services, stream_id: &str) -> Option<(i64, std::path::PathBuf)> {
    let streams = svc.streams.list_streams().await.ok()?;
    streams
        .into_iter()
        .find(|s| s.id.to_string() == stream_id)
        .map(|s| (s.id.value(), std::path::PathBuf::from(&s.worktree_path)))
}

/// Apply one session event. Returns the stream whose diagnostics changed
/// and the files that changed.
pub async fn apply(svc: &Services, event: LspSessionEvent) -> Option<(i64, Vec<String>)> {
    match event {
        LspSessionEvent::ServerNotification {
            stream_id,
            language,
            method,
            params,
        } if method == "textDocument/publishDiagnostics" => {
            let (id, worktree) = stream_of(svc, &stream_id).await?;
            let (path, rows) = parse_publish(&params, &worktree)?;
            if let Err(e) = svc
                .diagnostic_store
                .replace_file(id, language, path.clone(), rows)
                .await
            {
                tracing::warn!(error = %e, "storing LSP diagnostics failed");
                return None;
            }
            Some((id, vec![path]))
        }
        LspSessionEvent::SessionStatus {
            stream_id,
            language,
            status,
            ..
        } if !matches!(status, LspSessionStatus::Ready) => {
            // Restarted / Crashed / Stopped: what it said no longer holds
            // (a restarted server republishes as files reopen).
            let (id, _) = stream_of(svc, &stream_id).await?;
            let paths = svc.diagnostic_store.clear_server(id, language).await.ok()?;
            Some((id, paths))
        }
        _ => None,
    }
}

/// No server runs yet, so what the last process's servers said no longer
/// holds: clear it, and log each cleared file at zero (as a crash does).
pub async fn clear_at_boot(svc: &Services) {
    let cleared = match svc.diagnostic_store.clear_all().await {
        Ok(cleared) => cleared,
        Err(e) => {
            tracing::warn!(error = %e, "clearing LSP diagnostics at boot failed");
            return;
        }
    };
    let mut by_stream: std::collections::BTreeMap<i64, BTreeSet<String>> = Default::default();
    for (stream_id, path) in cleared {
        by_stream.entry(stream_id).or_default().insert(path);
    }
    for (stream_id, paths) in by_stream {
        announce(svc, stream_id, paths).await;
    }
}

/// Clear stale rows, then persist publishes for the life of the process.
pub fn spawn(svc: std::sync::Arc<Services>) {
    let mut rx = svc.lsp_sessions.subscribe();
    tokio::spawn(async move {
        clear_at_boot(&svc).await;
        // The stream with unannounced changes, when to announce them, and
        // the files that changed. The deadline is fixed by the first
        // change, so a server that publishes continuously can't starve the
        // announcement.
        let mut pending: Option<(i64, tokio::time::Instant, BTreeSet<String>)> = None;
        loop {
            let event = match &pending {
                Some((_, deadline, _)) => {
                    match tokio::time::timeout_at(*deadline, rx.recv()).await {
                        Ok(e) => e,
                        Err(_) => {
                            if let Some((stream_id, _, paths)) = pending.take() {
                                announce(&svc, stream_id, paths).await;
                            }
                            continue;
                        }
                    }
                }
                None => rx.recv().await,
            };
            match event {
                Ok(e) => {
                    let Some((id, paths)) = apply(&svc, e).await else {
                        continue;
                    };
                    match &mut pending {
                        Some((p, _, pending_paths)) if *p == id => pending_paths.extend(paths),
                        // A second stream in the window: announce the first now.
                        _ => {
                            if let Some((p, _, pending_paths)) = pending.take() {
                                announce(&svc, p, pending_paths).await;
                            }
                            pending = Some((
                                id,
                                tokio::time::Instant::now() + DEBOUNCE,
                                paths.into_iter().collect(),
                            ));
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Announce a stream's changed diagnostics: the in-memory event, and
/// `code.diagnostics.changed@1` per file with its counts now.
async fn announce(svc: &Services, stream_id: i64, paths: BTreeSet<String>) {
    use oxplow_domain::events::schema::{CodeDiagnosticsChanged, CodeDiagnosticsChangedV1};
    let stream = oxplow_domain::StreamId::new(stream_id);
    for path in paths {
        let counts = match svc.diagnostic_store.counts(stream_id, path.clone()).await {
            Ok(counts) => counts,
            Err(e) => {
                tracing::warn!(error = %e, path, "reading diagnostic counts failed");
                continue;
            }
        };
        let env = oxplow_domain::Envelope::typed::<CodeDiagnosticsChanged>(
            oxplow_domain::refs::build::system_source("lsp_diagnostics"),
            &CodeDiagnosticsChangedV1 {
                stream: format!("stream:{stream}"),
                path: path.clone(),
                counts,
            },
        )
        .with_anchors(oxplow_domain::Anchors {
            stream_id: Some(stream),
            ..oxplow_domain::Anchors::default()
        })
        .with_subject([format!("file:{path}")]);
        if let Err(e) = svc.event_log_store.append(env).await {
            tracing::warn!(error = %e, path, "logging code.diagnostics.changed failed");
        }
    }
    svc.events
        .emit(OxplowEvent::DiagnosticsChanged { stream_id });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn publish(uri: &str) -> Value {
        json!({
            "uri": uri,
            "diagnostics": [
                {"range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 9}},
                 "severity": 1, "code": "E0308", "source": "rustc", "message": "mismatched types"},
                {"range": {"start": {"line": 9, "character": 0}, "end": {"line": 10, "character": 1}},
                 "severity": 2, "code": 6133, "message": "unused"},
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                 "message": "no severity"}
            ]
        })
    }

    #[test]
    fn publish_diagnostics_become_repo_relative_one_based_rows() {
        let (path, rows) =
            parse_publish(&publish("file:///repo/src/a%20b.rs"), Path::new("/repo")).unwrap();
        assert_eq!(path, "src/a b.rs");
        assert_eq!(
            rows[0],
            DiagnosticRow {
                severity: "error".into(),
                message: "mismatched types".into(),
                source: Some("rustc".into()),
                code: Some("E0308".into()),
                line: 3,
                col: 5,
                end_line: 3,
                end_col: 10,
            }
        );
        assert_eq!(
            (
                rows[1].severity.as_str(),
                rows[1].code.as_deref(),
                rows[1].source.as_deref()
            ),
            ("warning", Some("6133"), None)
        );
        assert_eq!(rows[2].severity, "error");
    }

    #[test]
    fn files_outside_the_worktree_and_non_file_uris_are_ignored() {
        assert!(parse_publish(&publish("file:///elsewhere/x.rs"), Path::new("/repo")).is_none());
        assert!(parse_publish(&publish("untitled:Untitled-1"), Path::new("/repo")).is_none());
    }

    #[test]
    fn an_empty_publish_clears_the_file() {
        let (path, rows) = parse_publish(
            &json!({"uri": "file:///repo/a.rs", "diagnostics": []}),
            Path::new("/repo"),
        )
        .unwrap();
        assert_eq!((path.as_str(), rows.len()), ("a.rs", 0));
    }

    /// tsk571: the boot clear logs each cleared file at zero, like a
    /// crash does — the log never keeps counts the rows no longer hold.
    #[tokio::test]
    async fn the_boot_clear_logs_each_cleared_file_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Services::in_memory(dir.path()).unwrap();
        let stream = svc.streams.ensure_primary().await.unwrap();
        for path in ["a.rs", "b.rs"] {
            svc.diagnostic_store
                .replace_file(
                    stream.id.value(),
                    "rust".into(),
                    path.into(),
                    vec![oxplow_db::DiagnosticRow {
                        severity: "error".into(),
                        message: "bad".into(),
                        line: 1,
                        col: 1,
                        end_line: 1,
                        end_col: 2,
                        ..Default::default()
                    }],
                )
                .await
                .unwrap();
        }
        clear_at_boot(&svc).await;
        let logged: Vec<Value> = svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == "code.diagnostics.changed")
            .map(|e| e.envelope.payload)
            .collect();
        assert_eq!(logged.len(), 2, "{logged:?}");
        assert_eq!(logged[0]["path"], "a.rs");
        assert_eq!(logged[1]["path"], "b.rs");
        assert!(
            logged.iter().all(|e| e["counts"]["error"] == 0),
            "{logged:?}"
        );
    }

    #[tokio::test]
    async fn published_diagnostics_land_in_v_diagnostic_and_a_restart_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = std::sync::Arc::new(Services::in_memory(dir.path()).unwrap());
        let stream = svc.streams.ensure_primary().await.unwrap();
        let mut events = svc.events.subscribe();
        spawn(svc.clone());
        let uri =
            url::Url::from_file_path(std::path::Path::new(&stream.worktree_path).join("src/a.rs"))
                .unwrap()
                .to_string();
        let count = || async {
            let r = crate::sql_gateway::SqlGateway::new(svc.db.clone())
                .query_sql(
                    "SELECT path, severity, line FROM v_diagnostic",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(r.rows).unwrap()
        };
        let changed = |events: &mut tokio::sync::broadcast::Receiver<OxplowEvent>| {
            let id = stream.id.value();
            let mut rx = events.resubscribe();
            async move {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let Ok(OxplowEvent::DiagnosticsChanged { stream_id }) = rx.recv().await {
                            if stream_id == id {
                                return;
                            }
                        }
                    }
                })
                .await
                .expect("DiagnosticsChanged");
            }
        };
        // The subscriber may not be listening yet: publish until it lands.
        let wait = changed(&mut events);
        let publisher = {
            let svc = svc.clone();
            let sid = stream.id.to_string();
            tokio::spawn(async move {
                loop {
                    svc.lsp_sessions.emit_event_for_tests(LspSessionEvent::ServerNotification {
                        stream_id: sid.clone(),
                        language: "rust".into(),
                        method: "textDocument/publishDiagnostics".into(),
                        params: serde_json::json!({"uri": uri, "diagnostics": [
                            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                             "severity": 1, "message": "bad"}]}),
                    });
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };
        wait.await;
        publisher.abort();
        assert_eq!(count().await, serde_json::json!([["src/a.rs", "error", 1]]));
        let logged = || async {
            svc.event_log_store
                .read_after(0, 1000)
                .await
                .unwrap()
                .into_iter()
                .filter(|e| e.envelope.event_type == "code.diagnostics.changed")
                .map(|e| e.envelope.payload)
                .collect::<Vec<_>>()
        };
        let first = logged().await;
        assert_eq!(first[0]["path"], "src/a.rs");
        assert_eq!(first[0]["stream"], format!("stream:{}", stream.id));
        assert_eq!(first[0]["counts"]["error"], 1);

        let wait = changed(&mut events);
        svc.lsp_sessions
            .emit_event_for_tests(LspSessionEvent::SessionStatus {
                stream_id: stream.id.to_string(),
                language: "rust".into(),
                status: LspSessionStatus::Restarted,
                message: None,
            });
        wait.await;
        assert_eq!(count().await, serde_json::json!([]));
        // The server's reports went with it, and the log says so.
        let last = logged().await.pop().unwrap();
        assert_eq!(last["path"], "src/a.rs");
        assert_eq!(last["counts"]["error"], 0);
    }
}
