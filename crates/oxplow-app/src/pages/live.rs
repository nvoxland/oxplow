//! A loader's `live: <column>`: the rows of a list of places (the ones a
//! person bookmarked or visited, so Go To) are kept only while what they
//! name still exists. A removed extension page, a deleted file or a
//! knowledge page that is gone is left out, so the list never offers a link
//! that opens on nothing.
//!
//! Only what can be told gone is: a file or folder missing from the
//! stream's worktree, a page no enabled extension has, a record whose
//! resource resolves through a view that has no row for it. A URI of a
//! resource nothing resolves (a settings area, a terminal) and a web URL are
//! kept: absent proof, a row stays.

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::uri::{resources as r, OxplowUri};

use crate::Services;

/// Keep the rows of `result` whose `column` URI isn't gone, in `stream`'s
/// worktree (the primary checkout for none).
pub(crate) async fn keep_live(
    svc: &Services,
    stream: Option<i64>,
    result: &mut SqlQueryResult,
    column: &str,
) -> Result<(), String> {
    let Some(at) = result.columns.iter().position(|c| c == column) else {
        return Err(format!(
            "`live: {column}` names a column the loader doesn't return"
        ));
    };
    let root = match stream {
        Some(stream) => crate::change_analysis::stream_root(&svc.worktrees, stream).await,
        None => crate::collector_runner::primary_root(&svc.worktrees).await,
    };
    let mut kept = Vec::with_capacity(result.rows.len());
    for row in std::mem::take(&mut result.rows) {
        let gone = match row.get(at) {
            Some(SqlCell::Text(uri)) => is_gone(svc, &root, uri).await,
            _ => false,
        };
        if !gone {
            kept.push(row);
        }
    }
    result.rows = kept;
    Ok(())
}

/// Whether what `uri` names is known to be gone.
pub(crate) async fn is_gone(svc: &Services, root: &std::path::Path, uri: &str) -> bool {
    let Ok(parsed) = OxplowUri::parse(uri) else {
        return false;
    };
    // Another filesystem's file isn't in this worktree.
    if parsed
        .param("filesystem")
        .is_some_and(|f| f != oxplow_domain::filesystem::LOCAL)
    {
        return false;
    }
    let resource = parsed.resource().to_string();
    match resource.as_str() {
        r::FILE => match parsed.param("path") {
            Some(path) => !exists(root, path, false).await,
            None => false,
        },
        r::DIR => match parsed.param("path") {
            Some(path) => !exists(root, path, true).await,
            None => false,
        },
        r::PAGE => !page_exists(svc, &parsed.identity()).await,
        _ => {
            // A resource whose declaration resolves it through a view: gone
            // when the view has no row for the URI. Any other is kept.
            let declared = crate::ref_resolver::resolve_view(svc, &resource);
            declared.is_some() && !crate::ref_resolver::resolve_ref(svc, uri).await.resolved
        }
    }
}

async fn exists(root: &std::path::Path, path: &str, dir: bool) -> bool {
    // An empty path is the root; a path outside the tree isn't ours to judge.
    if path.is_empty() || path.split('/').any(|part| part == "..") {
        return true;
    }
    match tokio::fs::metadata(root.join(path)).await {
        Ok(meta) if dir => meta.is_dir(),
        Ok(meta) => meta.is_file(),
        Err(_) => false,
    }
}

async fn page_exists(svc: &Services, uri: &str) -> bool {
    svc.sql
        .query_sql(
            "SELECT 1 FROM v_page WHERE uri = ?1 LIMIT 1",
            vec![SqlCell::Text(uri.to_string())],
            None,
        )
        .await
        .map(|out| !out.rows.is_empty())
        // Can't tell: keep it.
        .unwrap_or(true)
}
