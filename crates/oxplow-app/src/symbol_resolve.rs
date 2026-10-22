//! Where a symbol URI points (`.context/language-support.md` "Symbols"):
//! the current symbol is its `v_symbol` row; a URI pinned to a version
//! (`rev=snap:<id>`, `rev=git:<sha>`) is found by extracting the file as it was
//! then. A pin whose version is gone — a pruned snapshot, an unknown
//! commit — or no longer holds the symbol falls back to the current one,
//! flagged, rather than a dead link.

use std::path::Path;

use oxplow_domain::uri::build;
use oxplow_domain::uri::resources::SYMBOL;
use oxplow_domain::vcs::Revision;
use oxplow_domain::{DomainError, StreamId};
use serde::{Deserialize, Serialize};

use crate::Services;

/// Where a symbol is: its file and the line/column of its name (1-based),
/// at `revision` when the ref pinned one that still holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SymbolLocation {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub revision: Option<Revision>,
    /// The ref pinned a version that's gone (or no longer holds it): this
    /// is the current symbol instead.
    pub version_gone: bool,
}

/// Where `r` points in `stream` (working in `ws`), or `None` when it isn't
/// a symbol there now (and its pinned version, if any, doesn't have it
/// either).
pub async fn resolve(
    svc: &Services,
    stream: StreamId,
    ws: &Path,
    r: &str,
) -> Result<Option<SymbolLocation>, DomainError> {
    let parsed = build::parse_resource(r, SYMBOL)
        .map_err(|_| DomainError::Invalid(format!("`{r}` isn't a symbol URI")))?;
    let (Some(path), Some(symbol)) = (parsed.param("path"), parsed.param("symbol")) else {
        return Err(DomainError::Invalid(format!(
            "`{r}` names no path and symbol"
        )));
    };
    let mut version_gone = false;
    if let Some(rev) = parsed.param("rev") {
        let revision: Revision = rev
            .parse()
            .map_err(|e: String| DomainError::Invalid(format!("`{r}`: {e}")))?;
        match at_revision(svc, ws, path, symbol, &revision).await {
            Some(found) => return Ok(Some(found)),
            None => version_gone = true,
        }
    }
    let current = build::symbol_uri(path, symbol);
    let id = stream.value();
    let row = svc
        .db
        .read(move |tx| {
            tx.query_row(
                "SELECT path, line, col FROM symbol WHERE stream_id = ?1 AND uri = ?2",
                rusqlite::params![id, current],
                |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
            .map_err(oxplow_db::map_sql_err)
        })
        .await?;
    Ok(row.map(|(path, line, col)| SymbolLocation {
        path,
        line,
        col,
        revision: None,
        version_gone,
    }))
}

/// `symbol` in the file at `path`, found as the file was at `revision`.
async fn at_revision(
    svc: &Services,
    ws: &Path,
    path: &str,
    symbol: &str,
    revision: &Revision,
) -> Option<SymbolLocation> {
    let Ok(Some(bytes)) = svc.trees.read_at(ws, revision, path).await else {
        return None;
    };
    let text = String::from_utf8(bytes).ok()?;
    let found = oxplow_code_metrics::extract(path, &text)
        .into_iter()
        .find(|d| d.id == symbol)?;
    Some(SymbolLocation {
        path: path.to_string(),
        line: found.name_line,
        col: found.name_col,
        revision: Some(revision.clone()),
        version_gone: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A current ref finds its row; a pin to a live snapshot finds the
    /// symbol where it was then; a pin to a snapshot that's gone falls back
    /// to the current symbol, flagged.
    #[tokio::test]
    async fn a_symbol_ref_resolves_now_at_its_version_and_past_a_gone_version() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Services::in_memory(dir.path()).await.unwrap();
        let stream = svc.streams.ensure_primary().await.unwrap().id;
        let old = "class Widget:\n    def spin(self): pass\n";
        let snap = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        svc.snapshot_store
            .capture(oxplow_db::FileSnapshot {
                id: 0,
                stream_id: stream,
                path: "Lib/w.py".into(),
                blob_hash: Some(svc.blobs.write(old.as_bytes()).unwrap()),
                size_bytes: old.len() as i64,
                captured_at: oxplow_domain::Timestamp::from_unix_ms(0),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: Some(0),
                content_hash: None,
            })
            .await
            .unwrap();
        // Now the method sits two lines lower.
        let rows = vec![oxplow_db::SymbolRow {
            id: "Widget::spin".into(),
            name: "spin".into(),
            kind: "method".into(),
            container: Some("Widget".into()),
            signature: None,
            line: 4,
            col: 9,
            start_line: 4,
            start_col: 5,
            end_line: 4,
            end_col: 25,
        }];
        svc.symbol_store
            .record(
                stream.value(),
                snap + 1,
                vec![oxplow_db::FileSymbols {
                    path: "Lib/w.py".into(),
                    language: "python".into(),
                    rows: Some(rows),
                }],
                oxplow_db::SymbolCapture::default(),
            )
            .await
            .unwrap();

        let now = resolve(
            &svc,
            stream,
            dir.path(),
            "oxplow:oxplow/symbol?path=Lib/w.py&symbol=Widget::spin",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!((now.path.as_str(), now.line, now.col), ("Lib/w.py", 4, 9));
        assert!(!now.version_gone && now.revision.is_none());

        let then = resolve(
            &svc,
            stream,
            dir.path(),
            &format!("oxplow:oxplow/symbol?path=Lib/w.py&rev=snap:{snap}&symbol=Widget::spin"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!((then.line, then.col), (2, 9));
        assert_eq!(then.revision, Some(Revision::Snapshot(snap)));

        let gone = resolve(
            &svc,
            stream,
            dir.path(),
            "oxplow:oxplow/symbol?path=Lib/w.py&rev=snap:999999&symbol=Widget::spin",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!((gone.line, gone.version_gone), (4, true));

        assert!(resolve(
            &svc,
            stream,
            dir.path(),
            "oxplow:oxplow/symbol?path=Lib/w.py&symbol=Nope"
        )
        .await
        .unwrap()
        .is_none());
    }
}
