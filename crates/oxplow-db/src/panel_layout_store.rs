//! The person's left-nav layout (`panel_layout`, P6.G1): three lists, the
//! project's, each stream's and each thread's (`visibility_scope`). A
//! viewer — a thread and its stream — sees its thread's panels, then its
//! stream's, then the project's (`.context/pages-and-tabs.md` "Left-nav
//! panels"). Panels are named by id (`ext:<extension>/<panel>`); one no
//! list names shows expanded at the end of the project's. Each device has its
//! own three lists, and the rules hold within one device's. Local to this
//! project's database, never the repo.
//!
//! The rules, kept on every write:
//! - **One group per viewer.** A panel is in at most one of the lists a
//!   viewer sees. In the project's (shown) it is in no other list
//!   anywhere; in a stream's, in none of that stream's threads'.
//! - **The project list names every panel placed.** One in a thread's or
//!   stream's list keeps a hidden project row, so it isn't new for
//!   another viewer. Only a project row can be hidden.

use oxplow_domain::{DeviceId, DomainError, StreamId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::visibility_scope::{Viewer, VisibilityScope};
use crate::Database;

/// One panel's place in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PanelPlacement {
    pub panel: String,
    /// Only in the project's list: placed, but not shown there.
    pub hidden: bool,
    pub collapsed: bool,
}

/// The lists a viewer sees, each in order: its thread's, its stream's and
/// the project's (with its hidden rows).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScopedPanelLayout {
    pub thread: Vec<PanelPlacement>,
    pub stream: Vec<PanelPlacement>,
    pub project: Vec<PanelPlacement>,
}

pub struct SqlitePanelLayoutStore {
    db: Database,
}

impl SqlitePanelLayoutStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// What `viewer` sees on `device`.
    pub async fn get(
        &self,
        device: &DeviceId,
        viewer: Viewer,
    ) -> Result<ScopedPanelLayout, DomainError> {
        let device = device.clone();
        self.db.read(move |c| get_tx(c, &device, viewer)).await
    }

    /// Replace the lists `viewer` sees on `device` with `layout`, each in its
    /// order, and keep the rules (module doc) for the device's other viewers.
    pub async fn set(
        &self,
        device: &DeviceId,
        viewer: Viewer,
        layout: ScopedPanelLayout,
    ) -> Result<(), DomainError> {
        let device = device.clone();
        self.db
            .transaction(move |tx| set_tx(tx, &device, viewer, &layout))
            .await
    }
}

/// The viewer of `thread` (its own stream) or of `stream` alone.
pub fn viewer_tx(
    conn: &Connection,
    thread: Option<oxplow_domain::ThreadId>,
    stream: Option<StreamId>,
) -> Result<Viewer, DomainError> {
    let Some(t) = thread else {
        return Ok(Viewer {
            thread: None,
            stream,
        });
    };
    let s: Option<i64> = conn
        .query_row(
            "SELECT stream_id FROM threads WHERE id = ?1",
            [t.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    let s = s.ok_or_else(|| DomainError::Invalid(format!("no thread `{t}`")))?;
    Ok(Viewer {
        thread: Some(t),
        stream: Some(StreamId::new(s)),
    })
}

fn list_tx(
    conn: &Connection,
    device: &DeviceId,
    scope: VisibilityScope,
    owner: Option<i64>,
) -> Result<Vec<PanelPlacement>, DomainError> {
    let column = match scope {
        VisibilityScope::Thread => "thread_id = ?2",
        VisibilityScope::Stream => "stream_id = ?2",
        VisibilityScope::Project => "?2 IS NULL",
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT panel, hidden, collapsed FROM panel_layout
             WHERE device_id = ?3 AND visibility_scope = ?1 AND {column} ORDER BY position"
        ))
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![scope.as_str(), owner, device.as_str()], |r| {
            Ok(PanelPlacement {
                panel: r.get(0)?,
                hidden: r.get::<_, i64>(1)? == 1,
                collapsed: r.get::<_, i64>(2)? == 1,
            })
        })
        .map_err(map_sql_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)?;
    Ok(rows)
}

pub fn get_tx(
    conn: &Connection,
    device: &DeviceId,
    viewer: Viewer,
) -> Result<ScopedPanelLayout, DomainError> {
    Ok(ScopedPanelLayout {
        thread: match viewer.thread {
            Some(t) => list_tx(conn, device, VisibilityScope::Thread, Some(t.value()))?,
            None => Vec::new(),
        },
        stream: match viewer.stream {
            Some(s) => list_tx(conn, device, VisibilityScope::Stream, Some(s.value()))?,
            None => Vec::new(),
        },
        project: list_tx(conn, device, VisibilityScope::Project, None)?,
    })
}

pub fn set_tx(
    conn: &Connection,
    device: &DeviceId,
    viewer: Viewer,
    layout: &ScopedPanelLayout,
) -> Result<(), DomainError> {
    let device = device.as_str();
    // What the viewer would see: each panel once.
    let mut seen = std::collections::HashSet::new();
    let shown = layout
        .thread
        .iter()
        .chain(&layout.stream)
        .chain(layout.project.iter().filter(|p| !p.hidden));
    for p in shown {
        if !seen.insert(p.panel.as_str()) {
            return Err(DomainError::Invalid(format!(
                "panel `{}` is in two of the lists one viewer sees",
                p.panel
            )));
        }
    }
    let mut project = layout.project.clone();
    for p in layout.thread.iter().chain(&layout.stream) {
        if p.hidden {
            return Err(DomainError::Invalid(format!(
                "panel `{}`: only a project row can be hidden",
                p.panel
            )));
        }
        // Placed in a narrower list: its project row is kept, hidden.
        if !project.iter().any(|q| q.panel == p.panel) {
            project.push(PanelPlacement {
                panel: p.panel.clone(),
                hidden: true,
                collapsed: false,
            });
        }
    }
    let (thread_id, _) = if layout.thread.is_empty() {
        (viewer.thread.map(|t| t.value()), None)
    } else {
        viewer.owner(VisibilityScope::Thread, "panel")?
    };
    let (_, stream_id) = if layout.stream.is_empty() {
        (None, viewer.stream.map(|s| s.value()))
    } else {
        viewer.owner(VisibilityScope::Stream, "panel")?
    };

    conn.execute(
        "DELETE FROM panel_layout WHERE device_id = ?1 AND visibility_scope = 'project'",
        [device],
    )
    .map_err(map_sql_err)?;
    if let Some(t) = thread_id {
        conn.execute(
            "DELETE FROM panel_layout WHERE device_id = ?2 AND visibility_scope = 'thread' AND thread_id = ?1",
            params![t, device],
        )
        .map_err(map_sql_err)?;
    }
    if let Some(s) = stream_id {
        conn.execute(
            "DELETE FROM panel_layout WHERE device_id = ?2 AND visibility_scope = 'stream' AND stream_id = ?1",
            params![s, device],
        )
        .map_err(map_sql_err)?;
    }
    let insert = |scope: VisibilityScope, list: &[PanelPlacement]| -> Result<(), DomainError> {
        let (t, s) = match scope {
            VisibilityScope::Thread => (thread_id, None),
            VisibilityScope::Stream => (None, stream_id),
            VisibilityScope::Project => (None, None),
        };
        for (i, p) in list.iter().enumerate() {
            conn.execute(
                "INSERT INTO panel_layout (device_id, panel, visibility_scope, thread_id, stream_id, position, hidden, collapsed)
                 VALUES (?8, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![p.panel, scope.as_str(), t, s, i as i64, p.hidden as i64, p.collapsed as i64, device],
            )
            .map_err(map_sql_err)?;
        }
        Ok(())
    };
    insert(VisibilityScope::Thread, &layout.thread)?;
    insert(VisibilityScope::Stream, &layout.stream)?;
    insert(VisibilityScope::Project, &project)?;

    // The rules for every other viewer.
    for p in project.iter().filter(|p| !p.hidden) {
        conn.execute(
            "DELETE FROM panel_layout WHERE device_id = ?2 AND panel = ?1 AND visibility_scope <> 'project'",
            params![p.panel, device],
        )
        .map_err(map_sql_err)?;
    }
    if let Some(s) = stream_id {
        for p in &layout.stream {
            conn.execute(
                "DELETE FROM panel_layout
                 WHERE device_id = ?3 AND panel = ?1 AND visibility_scope = 'thread'
                   AND thread_id IN (SELECT id FROM threads WHERE stream_id = ?2)",
                params![p.panel, s, device],
            )
            .map_err(map_sql_err)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ThreadId;

    /// The device every test but one places panels on.
    fn mac() -> DeviceId {
        DeviceId::parse("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427").unwrap()
    }

    fn shown(panel: &str) -> PanelPlacement {
        PanelPlacement {
            panel: panel.into(),
            hidden: false,
            collapsed: false,
        }
    }

    fn hidden(panel: &str) -> PanelPlacement {
        PanelPlacement {
            hidden: true,
            ..shown(panel)
        }
    }

    fn names(list: &[PanelPlacement]) -> Vec<&str> {
        list.iter().map(|p| p.panel.as_str()).collect()
    }

    /// Streams 1..=3 (X, Y, Z); X has threads 1 and 3 (A and C) and 4,
    /// Y has thread 2, Z has thread 5.
    fn seed(conn: &Connection) {
        for s in 1..=3 {
            conn.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                 VALUES (?1, CASE ?1 WHEN 1 THEN 'primary' ELSE 'worktree' END, 's', 'main', 'refs/heads/main', 'local', '/tmp', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![s],
            )
            .unwrap();
        }
        for (t, s) in [(1, 1), (2, 2), (3, 1), (4, 1), (5, 3)] {
            conn.execute(
                "INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                 VALUES (?1, ?2, 't', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![t, s],
            )
            .unwrap();
        }
    }

    fn at(conn: &Connection, thread: i64) -> Viewer {
        viewer_tx(conn, Some(ThreadId::new(thread)), None).unwrap()
    }

    async fn store() -> (SqlitePanelLayoutStore, Database) {
        let db = Database::in_memory();
        db.transaction(|c| {
            seed(c);
            Ok(())
        })
        .await
        .unwrap();
        (SqlitePanelLayoutStore::new(db.clone()), db)
    }

    /// One panel in the thread lists of A and C of stream X, in stream Y's
    /// list, and nowhere in stream Z: each viewer sees its own, and a
    /// thread-only panel is placed (hidden), not new, everywhere else.
    #[tokio::test]
    async fn a_panel_can_be_on_some_threads_and_some_streams() {
        let (store, db) = store().await;
        let v = |t| {
            let db = db.clone();
            async move { db.read(move |c| Ok(at(c, t))).await.unwrap() }
        };
        let (a, y, c, other_x, z) = (v(1).await, v(2).await, v(3).await, v(4).await, v(5).await);
        let mut base = store.get(&mac(), a).await.unwrap();
        base.project = vec![shown("go")];
        base.thread = vec![shown("p")];
        store.set(&mac(), a, base).await.unwrap();
        let mut on_c = store.get(&mac(), c).await.unwrap();
        on_c.thread = vec![shown("p")];
        store.set(&mac(), c, on_c).await.unwrap();
        let mut on_y = store.get(&mac(), y).await.unwrap();
        on_y.stream = vec![shown("p")];
        store.set(&mac(), y, on_y).await.unwrap();

        for (viewer, thread, stream) in [
            (a, vec!["p"], vec![]),
            (c, vec!["p"], vec![]),
            (y, vec![], vec!["p"]),
        ] {
            let got = store.get(&mac(), viewer).await.unwrap();
            assert_eq!((names(&got.thread), names(&got.stream)), (thread, stream));
        }
        for viewer in [other_x, z] {
            let got = store.get(&mac(), viewer).await.unwrap();
            assert!(got.thread.is_empty() && got.stream.is_empty(), "{got:?}");
        }
        // Everyone's project list names it, hidden: placed, not new.
        assert_eq!(
            store.get(&mac(), z).await.unwrap().project,
            vec![shown("go"), hidden("p")]
        );
    }

    /// Shown in the project's list, a panel leaves every other list.
    #[tokio::test]
    async fn everywhere_takes_it_out_of_every_other_list() {
        let (store, db) = store().await;
        let (a, y) = db.read(|c| Ok((at(c, 1), at(c, 2)))).await.unwrap();
        store
            .set(
                &mac(),
                a,
                ScopedPanelLayout {
                    thread: vec![shown("p")],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut on_y = store.get(&mac(), y).await.unwrap();
        on_y.stream = vec![shown("p")];
        store.set(&mac(), y, on_y).await.unwrap();
        let mut from_a = store.get(&mac(), a).await.unwrap();
        from_a.thread.clear();
        from_a.project = vec![shown("p")];
        store.set(&mac(), a, from_a).await.unwrap();
        assert!(store.get(&mac(), y).await.unwrap().stream.is_empty());
        assert_eq!(
            store.get(&mac(), a).await.unwrap().project,
            vec![shown("p")]
        );
    }

    /// In a stream's list, a panel leaves that stream's threads' lists,
    /// and no other stream's.
    #[tokio::test]
    async fn a_stream_takes_it_out_of_its_threads() {
        let (store, db) = store().await;
        let (a, y, c) = db
            .read(|c| Ok((at(c, 1), at(c, 2), at(c, 3))))
            .await
            .unwrap();
        for viewer in [c, y] {
            let mut l = store.get(&mac(), viewer).await.unwrap();
            l.thread = vec![shown("p")];
            store.set(&mac(), viewer, l).await.unwrap();
        }
        let mut on_x = store.get(&mac(), a).await.unwrap();
        on_x.stream = vec![shown("p")];
        store.set(&mac(), a, on_x).await.unwrap();
        assert!(store.get(&mac(), c).await.unwrap().thread.is_empty());
        assert_eq!(
            names(&store.get(&mac(), c).await.unwrap().stream),
            vec!["p"]
        );
        assert_eq!(
            names(&store.get(&mac(), y).await.unwrap().thread),
            vec!["p"]
        );
    }

    /// A viewer sees a panel once; a narrower row is never hidden; a
    /// thread list needs a thread.
    #[tokio::test]
    async fn what_a_viewer_sees_is_each_panel_once() {
        let (store, db) = store().await;
        let a = db.read(|c| Ok(at(c, 1))).await.unwrap();
        for bad in [
            ScopedPanelLayout {
                thread: vec![shown("p")],
                project: vec![shown("p")],
                ..Default::default()
            },
            ScopedPanelLayout {
                thread: vec![shown("p")],
                stream: vec![shown("p")],
                ..Default::default()
            },
            ScopedPanelLayout {
                thread: vec![hidden("p")],
                ..Default::default()
            },
        ] {
            let err = store.set(&mac(), a, bad).await.unwrap_err();
            assert!(matches!(err, DomainError::Invalid(_)), "{err:?}");
        }
        let err = store
            .set(
                &mac(),
                Viewer::default(),
                ScopedPanelLayout {
                    thread: vec![shown("p")],
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "{err:?}");
    }

    /// A closed-and-deleted thread takes its list with it; the project's
    /// list keeps order, hidden and collapsed.
    #[tokio::test]
    async fn the_project_list_persists_and_a_deleted_thread_takes_its_list() {
        let (store, db) = store().await;
        let a = db.read(|c| Ok(at(c, 1))).await.unwrap();
        let project = vec![
            PanelPlacement {
                collapsed: true,
                ..shown("comments")
            },
            shown("work"),
            hidden("prs"),
        ];
        store
            .set(
                &mac(),
                a,
                ScopedPanelLayout {
                    thread: vec![shown("t")],
                    project: project.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut want = project.clone();
        want.push(hidden("t"));
        assert_eq!(store.get(&mac(), a).await.unwrap().project, want);
        db.transaction(|c| {
            c.execute("DELETE FROM threads WHERE id = 1", [])
                .map_err(map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
        let n: i64 = db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM panel_layout WHERE visibility_scope = 'thread'",
                    [],
                    |r| r.get(0),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// Each device keeps its own lists: one device's placing changes nothing
    /// another sees.
    #[tokio::test]
    async fn each_device_keeps_its_own_layout() {
        let (store, db) = store().await;
        let a = db.read(move |c| Ok(at(c, 1))).await.unwrap();
        let tablet = DeviceId::generate();
        let mut on_mac = store.get(&mac(), a).await.unwrap();
        on_mac.project = vec![shown("go"), shown("tasks")];
        on_mac.thread = vec![shown("notes")];
        store.set(&mac(), a, on_mac.clone()).await.unwrap();
        let on_tablet = store.get(&tablet, a).await.unwrap();
        assert_eq!(
            on_tablet,
            ScopedPanelLayout::default(),
            "the tablet starts from the defaults"
        );
        store
            .set(
                &tablet,
                a,
                ScopedPanelLayout {
                    // `notes` is one of the Mac's thread panels: the tablet's
                    // rule for it must not reach the Mac's row.
                    project: vec![shown("notes")],
                    ..ScopedPanelLayout::default()
                },
            )
            .await
            .unwrap();
        let mac_now = store.get(&mac(), a).await.unwrap();
        assert_eq!(names(&mac_now.thread), vec!["notes"]);
        assert_eq!(names(&mac_now.project), vec!["go", "tasks", "notes"]);
    }
}
