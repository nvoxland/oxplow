//! Tabs an agent opened for a device that was away (`tab_queued`): kept in
//! the order they came, and read by the device when its window next
//! connects; a row goes once the window has kept the tab
//! (`.context/remote-daemon.md` "An agent's tab on a device").

use oxplow_domain::{DeviceId, DomainError, ThreadId, Timestamp};
use rusqlite::params;

use crate::database::map_sql_err;
use crate::Database;

/// Most tabs that wait for one device: the oldest goes for a new one.
pub const MAX_PER_DEVICE: i64 = 50;
/// A tab that waited longer than this is dropped, not opened.
pub const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 3600);

/// A tab waiting for a device: its row, the thread it opens in, the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedRow {
    pub id: i64,
    pub thread: ThreadId,
    pub uri: String,
}

pub struct TabQueueStore {
    db: Database,
}

impl TabQueueStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Whether this project has state for `device` (a stream, a thread or
    /// tabs it saved): a device id nobody ever used is not one to wait for.
    pub async fn knows(&self, device: &DeviceId) -> Result<bool, DomainError> {
        let device = device.as_str().to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM device_view_state WHERE device_id = ?1)
                         OR EXISTS (SELECT 1 FROM thread_selection WHERE device_id = ?1)
                         OR EXISTS (SELECT 1 FROM tab_session WHERE device_id = ?1)",
                    [&device],
                    |r| r.get(0),
                )
            })
            .await
    }

    /// `uri` waits for `device`, to open in `thread`; the same page waits
    /// once, and the oldest of more than [`MAX_PER_DEVICE`] goes.
    pub async fn queue(
        &self,
        device: &DeviceId,
        thread: &ThreadId,
        uri: &str,
    ) -> Result<(), DomainError> {
        let device = device.as_str().to_string();
        let thread = thread.value();
        let uri = uri.to_string();
        let now = Timestamp::now().to_string();
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT OR IGNORE INTO tab_queued (device_id, thread_id, uri, queued_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![device, thread, uri, now],
                )
                .map_err(map_sql_err)?;
                tx.execute(
                    "DELETE FROM tab_queued WHERE device_id = ?1 AND id NOT IN
                       (SELECT id FROM tab_queued WHERE device_id = ?1
                        ORDER BY id DESC LIMIT ?2)",
                    params![device, MAX_PER_DEVICE],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// What waits for `device`, oldest first. Rows that can no longer open
    /// (older than [`MAX_AGE`], a closed thread, an archived stream) go
    /// here; the others stay until [`TabQueueStore::forget`], so a tab is
    /// not lost when the window never keeps it.
    pub async fn take(&self, device: &DeviceId) -> Result<Vec<QueuedRow>, DomainError> {
        self.take_at(device, Timestamp::now()).await
    }

    async fn take_at(
        &self,
        device: &DeviceId,
        now: Timestamp,
    ) -> Result<Vec<QueuedRow>, DomainError> {
        let device = device.as_str().to_string();
        let cutoff =
            Timestamp::from_unix_ms(now.unix_ms() - MAX_AGE.as_millis() as i64).to_string();
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "DELETE FROM tab_queued WHERE device_id = ?1 AND (queued_at < ?2
                       OR thread_id NOT IN (
                            SELECT t.id FROM threads t JOIN streams s ON s.id = t.stream_id
                            WHERE t.closed_at IS NULL AND t.archived_at IS NULL
                              AND s.archived_at IS NULL))",
                    params![device, cutoff],
                )
                .map_err(map_sql_err)?;
                let mut stmt = tx
                    .prepare(
                        "SELECT id, thread_id, uri FROM tab_queued
                         WHERE device_id = ?1 ORDER BY id",
                    )
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map([&device], |r| {
                        Ok(QueuedRow {
                            id: r.get(0)?,
                            thread: ThreadId::new(r.get(1)?),
                            uri: r.get(2)?,
                        })
                    })
                    .map_err(map_sql_err)?
                    .collect::<Result<_, _>>()
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
    }

    /// The window kept these tabs: their rows go (only `device`'s own).
    pub async fn forget(&self, device: &DeviceId, ids: &[i64]) -> Result<(), DomainError> {
        let device = device.as_str().to_string();
        let ids = ids.to_vec();
        self.db
            .transaction(move |tx| {
                for id in &ids {
                    tx.execute(
                        "DELETE FROM tab_queued WHERE device_id = ?1 AND id = ?2",
                        params![device, id],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::stores::{StreamStore, ThreadStore};

    async fn thread(db: &Database) -> ThreadId {
        let at = Timestamp::from_unix_ms(1_700_000_000_000);
        let stream = oxplow_domain::Stream {
            id: oxplow_domain::StreamId::new(1),
            kind: oxplow_domain::StreamKind::Primary,
            title: "oxplow".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            host: oxplow_domain::HostId::LOCAL,
            custom_prompt: None,
            created_at: at,
            updated_at: at,
            archived_at: None,
        };
        crate::SqliteStreamStore::new(db.clone())
            .upsert(&stream)
            .await
            .unwrap();
        let thread = oxplow_domain::Thread {
            id: ThreadId::new(1),
            ..oxplow_domain::Thread::seed(stream.id, "explore", at)
        };
        crate::SqliteThreadStore::new(db.clone())
            .upsert(&thread)
            .await
            .unwrap()
    }

    fn uris(rows: &[QueuedRow]) -> Vec<&str> {
        rows.iter().map(|r| r.uri.as_str()).collect()
    }

    /// A device reads what waits for it, oldest first, until it forgets the
    /// rows; another device's tabs stay; the same page waits once.
    #[tokio::test]
    async fn a_device_reads_its_queued_tabs_until_it_forgets_them() {
        let db = Database::in_memory();
        let store = TabQueueStore::new(db.clone());
        let thread = thread(&db).await;
        let (mac, tablet) = (DeviceId::generate(), DeviceId::generate());
        assert!(store.take(&mac).await.unwrap().is_empty());
        store.queue(&tablet, &thread, "oxplow:a").await.unwrap();
        store.queue(&tablet, &thread, "oxplow:b").await.unwrap();
        store.queue(&tablet, &thread, "oxplow:a").await.unwrap();
        store.queue(&mac, &thread, "oxplow:c").await.unwrap();
        let rows = store.take(&tablet).await.unwrap();
        assert_eq!(uris(&rows), ["oxplow:a", "oxplow:b"]);
        assert_eq!(rows[0].thread, thread);
        assert_eq!(
            store.take(&tablet).await.unwrap(),
            rows,
            "a read keeps the rows: the tabs are not kept yet"
        );
        store.forget(&mac, &[rows[0].id]).await.unwrap();
        assert_eq!(store.take(&tablet).await.unwrap().len(), 2, "not its rows");
        store.forget(&tablet, &[rows[0].id]).await.unwrap();
        assert_eq!(uris(&store.take(&tablet).await.unwrap()), ["oxplow:b"]);
        assert_eq!(uris(&store.take(&mac).await.unwrap()), ["oxplow:c"]);
    }

    /// A device is known by a stream, a thread or tabs it has stored.
    #[tokio::test]
    async fn a_device_is_known_by_what_it_stored() {
        let db = Database::in_memory();
        let store = TabQueueStore::new(db.clone());
        let thread = thread(&db).await;
        let stranger = DeviceId::generate();
        assert!(!store.knows(&stranger).await.unwrap());
        for (table_sql, label) in [
            (
                "INSERT INTO device_view_state (device_id) VALUES (?1)",
                "view",
            ),
            (
                "INSERT INTO thread_selection (device_id, stream_id) VALUES (?1, 1)",
                "selection",
            ),
            (
                "INSERT INTO tab_session (device_id, thread_id, session_json, revision, updated_at)
                 VALUES (?1, 1, '{}', 1, 't')",
                "session",
            ),
        ] {
            let device = DeviceId::generate();
            let id = device.as_str().to_string();
            db.call(move |c| c.execute(table_sql, [&id])).await.unwrap();
            assert!(store.knows(&device).await.unwrap(), "{label}");
        }
        let _ = thread;
    }

    /// At most 50 wait for a device: the oldest goes.
    #[tokio::test]
    async fn the_oldest_tab_goes_past_the_limit() {
        let db = Database::in_memory();
        let store = TabQueueStore::new(db.clone());
        let thread = thread(&db).await;
        let device = DeviceId::generate();
        for n in 0..(MAX_PER_DEVICE + 3) {
            store
                .queue(&device, &thread, &format!("oxplow:{n}"))
                .await
                .unwrap();
        }
        let rows = store.take(&device).await.unwrap();
        assert_eq!(rows.len() as i64, MAX_PER_DEVICE);
        assert_eq!(rows[0].uri, "oxplow:3");
    }

    /// A tab that waited too long, one whose thread closed and one whose
    /// stream was archived are dropped when read, not opened.
    #[tokio::test]
    async fn a_tab_that_can_no_longer_open_is_dropped_when_read() {
        let db = Database::in_memory();
        let store = TabQueueStore::new(db.clone());
        let thread = thread(&db).await;
        let device = DeviceId::generate();
        for uri in ["oxplow:old", "oxplow:live"] {
            store.queue(&device, &thread, uri).await.unwrap();
        }
        let old = Timestamp::from_unix_ms(Timestamp::now().unix_ms() - 15 * 24 * 3600 * 1000);
        db.call(move |c| {
            c.execute(
                "UPDATE tab_queued SET queued_at = ?1 WHERE uri = 'oxplow:old'",
                [old.to_string()],
            )
        })
        .await
        .unwrap();
        assert_eq!(uris(&store.take(&device).await.unwrap()), ["oxplow:live"]);
        for (sql, why) in [
            ("UPDATE threads SET closed_at = 't'", "closed thread"),
            (
                "UPDATE threads SET closed_at = NULL; UPDATE streams SET archived_at = 't'",
                "archived stream",
            ),
        ] {
            db.call(move |c| c.execute_batch(sql)).await.unwrap();
            assert!(store.take(&device).await.unwrap().is_empty(), "{why}");
            db.call(|c| {
                c.execute_batch(
                    "UPDATE streams SET archived_at = NULL; UPDATE threads SET closed_at = NULL",
                )
            })
            .await
            .unwrap();
            store.queue(&device, &thread, "oxplow:live").await.unwrap();
        }
    }
}
