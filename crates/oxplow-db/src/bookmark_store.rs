//! The person's bookmarks (`bookmark`): pages they starred, each
//! at one scope — a thread's, a stream's or the project's. A viewer (a
//! thread, and its stream) sees its thread's bookmarks, its stream's and
//! the project's; a ref is bookmarked at most once across what one viewer
//! sees, so bookmarking it at another scope moves it. Read through
//! `v_bookmark`; every write is a `bookmark.*` command over these `_tx`
//! cores. See migration `V4__bookmark.sql`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use oxplow_domain::{DomainError, StreamId, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// Where a bookmark shows: the thread it was made in, that thread's
/// stream, or the whole project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkScope {
    Thread,
    Stream,
    Project,
}

impl BookmarkScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Stream => "stream",
            Self::Project => "project",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "thread" => Some(Self::Thread),
            "stream" => Some(Self::Stream),
            "project" => Some(Self::Project),
            _ => None,
        }
    }
}

/// Who's looking: the thread (if any) and its stream (if any). Decides
/// which bookmarks are visible, and owns a thread- or stream-scoped one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Viewer {
    pub thread: Option<ThreadId>,
    pub stream: Option<StreamId>,
}

/// One page bookmarked at a scope, as a viewer sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    /// The page's canonical ref (its tab id).
    #[serde(rename = "ref")]
    pub page_ref: String,
    pub page_kind: String,
    pub label: Option<String>,
    pub scope: BookmarkScope,
}

fn owner(viewer: Viewer, scope: BookmarkScope) -> Result<(Option<i64>, Option<i64>), DomainError> {
    match scope {
        BookmarkScope::Thread => viewer
            .thread
            .map(|t| (Some(t.value()), None))
            .ok_or_else(|| DomainError::Invalid("a thread bookmark needs a thread".into())),
        BookmarkScope::Stream => viewer
            .stream
            .map(|s| (None, Some(s.value())))
            .ok_or_else(|| DomainError::Invalid("a stream bookmark needs a stream".into())),
        BookmarkScope::Project => Ok((None, None)),
    }
}

/// The bookmark of `page_ref` the viewer sees, if any.
pub fn visible_tx(
    conn: &Connection,
    viewer: Viewer,
    page_ref: &str,
) -> Result<Option<Bookmark>, DomainError> {
    conn.query_row(
        "SELECT ref, page_kind, label, scope FROM bookmark
         WHERE ref = ?1
           AND ((scope = 'thread' AND thread_id = ?2)
                OR (scope = 'stream' AND stream_id = ?3)
                OR scope = 'project')
         ORDER BY CASE scope WHEN 'thread' THEN 0 WHEN 'stream' THEN 1 ELSE 2 END
         LIMIT 1",
        params![
            page_ref,
            viewer.thread.map(|t| t.value()),
            viewer.stream.map(|s| s.value())
        ],
        |row| {
            let scope: String = row.get(3)?;
            Ok(Bookmark {
                page_ref: row.get(0)?,
                page_kind: row.get(1)?,
                label: row.get(2)?,
                scope: BookmarkScope::parse(&scope).unwrap_or(BookmarkScope::Project),
            })
        },
    )
    .optional()
    .map_err(map_sql_err)
}

/// Take `page_ref` out of every scope the viewer sees; what it was, if any.
pub fn remove_tx(
    conn: &Connection,
    viewer: Viewer,
    page_ref: &str,
) -> Result<Option<Bookmark>, DomainError> {
    let before = visible_tx(conn, viewer, page_ref)?;
    conn.execute(
        "DELETE FROM bookmark
         WHERE ref = ?1
           AND ((scope = 'thread' AND thread_id = ?2)
                OR (scope = 'stream' AND stream_id = ?3)
                OR scope = 'project')",
        params![
            page_ref,
            viewer.thread.map(|t| t.value()),
            viewer.stream.map(|s| s.value())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(before)
}

/// Bookmark a page at `bookmark.scope` — moving it there if the viewer
/// already sees it at another; what it was before, if anything.
pub fn set_tx(
    conn: &Connection,
    viewer: Viewer,
    bookmark: &Bookmark,
) -> Result<Option<Bookmark>, DomainError> {
    let (thread_id, stream_id) = owner(viewer, bookmark.scope)?;
    let before = remove_tx(conn, viewer, &bookmark.page_ref)?;
    conn.execute(
        "INSERT INTO bookmark (ref, page_kind, label, scope, thread_id, stream_id, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            bookmark.page_ref,
            bookmark.page_kind,
            bookmark.label,
            bookmark.scope.as_str(),
            thread_id,
            stream_id,
            ts_to_string(Timestamp::now())
        ],
    )
    .map_err(map_sql_err)?;
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn bookmark(page_ref: &str, scope: BookmarkScope) -> Bookmark {
        Bookmark {
            page_ref: page_ref.into(),
            page_kind: "page".into(),
            label: Some("Git".into()),
            scope,
        }
    }

    /// Two streams with a thread each.
    fn seed(conn: &Connection) -> (Viewer, Viewer) {
        for s in 1..=2 {
            conn.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                 VALUES (?1, CASE ?1 WHEN 1 THEN 'primary' ELSE 'worktree' END, 's', 'main', 'refs/heads/main', 'local', '/tmp', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![s],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                 VALUES (?1, ?1, 't', 'active', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![s],
            )
            .unwrap();
        }
        let viewer = |n| Viewer {
            thread: Some(ThreadId::new(n)),
            stream: Some(StreamId::new(n)),
        };
        (viewer(1), viewer(2))
    }

    /// A ref is bookmarked once per viewer: bookmarking it at another scope
    /// moves it, and a thread's bookmark isn't another thread's.
    #[tokio::test]
    async fn bookmarking_again_moves_it_and_threads_see_their_own() {
        let db = Database::in_memory();
        db.transaction(|conn| {
            let (one, two) = seed(conn);
            assert_eq!(
                set_tx(conn, one, &bookmark("page:git", BookmarkScope::Thread))?,
                None
            );
            assert_eq!(visible_tx(conn, two, "page:git")?, None);
            let before = set_tx(conn, one, &bookmark("page:git", BookmarkScope::Stream))?;
            assert_eq!(before.map(|b| b.scope), Some(BookmarkScope::Thread));
            assert_eq!(
                visible_tx(conn, one, "page:git")?.map(|b| b.scope),
                Some(BookmarkScope::Stream)
            );
            assert_eq!(visible_tx(conn, two, "page:git")?, None);
            set_tx(conn, one, &bookmark("page:git", BookmarkScope::Project))?;
            assert_eq!(
                visible_tx(conn, two, "page:git")?.map(|b| b.scope),
                Some(BookmarkScope::Project)
            );
            let removed = remove_tx(conn, two, "page:git")?;
            assert_eq!(removed.map(|b| b.scope), Some(BookmarkScope::Project));
            assert_eq!(visible_tx(conn, one, "page:git")?, None);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A thread bookmark needs a thread to own it.
    #[tokio::test]
    async fn a_scope_needs_its_owner() {
        let db = Database::in_memory();
        let err = db
            .transaction(|conn| {
                set_tx(
                    conn,
                    Viewer::default(),
                    &bookmark("page:git", BookmarkScope::Thread),
                )
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "{err:?}");
    }
}
