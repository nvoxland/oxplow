//! Tabs an agent opened for a device that was away (`tab_queued`): kept in
//! the order they came, and taken by the device when its window next
//! connects (`.context/remote-daemon.md` "An agent's tab on a device").

use oxplow_domain::{DeviceId, DomainError, ThreadId, Timestamp};
use rusqlite::params;

use crate::database::map_sql_err;
use crate::Database;

pub struct TabQueueStore {
    db: Database,
}

impl TabQueueStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// `uri` waits for `device`, to open in `thread`.
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
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO tab_queued (device_id, thread_id, uri, queued_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![device, thread, uri, now],
                )?;
                Ok(())
            })
            .await
    }

    /// What waits for `device`, oldest first; read and removed in one
    /// transaction, so two windows never both get a tab.
    pub async fn take(&self, device: &DeviceId) -> Result<Vec<(ThreadId, String)>, DomainError> {
        let device = device.as_str().to_string();
        self.db
            .transaction(move |tx| {
                let rows: Vec<(i64, String)> = {
                    let mut stmt = tx
                        .prepare(
                            "SELECT thread_id, uri FROM tab_queued
                             WHERE device_id = ?1 ORDER BY id",
                        )
                        .map_err(map_sql_err)?;
                    let rows = stmt
                        .query_map([&device], |r| Ok((r.get(0)?, r.get(1)?)))
                        .map_err(map_sql_err)?
                        .collect::<Result<_, _>>()
                        .map_err(map_sql_err)?;
                    rows
                };
                tx.execute("DELETE FROM tab_queued WHERE device_id = ?1", [&device])
                    .map_err(map_sql_err)?;
                Ok(rows
                    .into_iter()
                    .map(|(thread, uri)| (ThreadId::new(thread), uri))
                    .collect())
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

    /// A device takes what waits for it, oldest first, once; another
    /// device's tabs stay.
    #[tokio::test]
    async fn a_device_takes_its_queued_tabs_once() {
        let db = Database::in_memory();
        let store = TabQueueStore::new(db.clone());
        let thread = thread(&db).await;
        let (mac, tablet) = (DeviceId::generate(), DeviceId::generate());
        assert!(store.take(&mac).await.unwrap().is_empty());
        store.queue(&tablet, &thread, "oxplow:a").await.unwrap();
        store.queue(&tablet, &thread, "oxplow:b").await.unwrap();
        store.queue(&mac, &thread, "oxplow:c").await.unwrap();
        assert_eq!(
            store.take(&tablet).await.unwrap(),
            vec![
                (thread, "oxplow:a".to_string()),
                (thread, "oxplow:b".to_string())
            ]
        );
        assert!(store.take(&tablet).await.unwrap().is_empty());
        assert_eq!(
            store.take(&mac).await.unwrap(),
            vec![(thread, "oxplow:c".to_string())]
        );
    }
}
