//! Each device's tabs (`tab_session`): for every thread the device opened, its
//! session of tabs (`apps/desktop/src/tabs/session.ts`), as the window keeps
//! it. Keyed by device and thread (`.context/data-model.md` "tab_session"):
//! another device of the same person has its own tabs, and a project's
//! database holds only its own threads' tabs.
//!
//! A save gives the revision it read: one that finds the row moved on since
//! is refused ([`TabSessionStore::save`] answers `None`), so a late save
//! never overwrites a newer one.

use oxplow_domain::{DeviceId, DomainError, ThreadId, Timestamp};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;

/// The most one thread's session may be, in bytes of JSON.
pub const MAX_SESSION_BYTES: usize = 256 * 1024;

/// A thread's tabs' revision: bumped by every save, and given back by the
/// next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
#[specta(transparent)]
pub struct TabRevision(#[specta(type = oxplow_domain::JsNumber)] pub i64);

/// What a save did: whether it wrote, and the row's revision now (the new
/// one; for a refused save, the one the row is at, `None` for no row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TabSave {
    pub saved: bool,
    pub revision: Option<TabRevision>,
}

/// One thread's stored tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TabSession {
    pub thread_id: ThreadId,
    /// The session, as the window writes it (JSON text).
    pub session_json: String,
    /// Bumped by every save: the next save gives it back.
    pub revision: TabRevision,
}

pub struct TabSessionStore {
    db: Database,
}

impl TabSessionStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Every thread's tabs `device` keeps.
    pub async fn list(&self, device: &DeviceId) -> Result<Vec<TabSession>, DomainError> {
        let device = device.as_str().to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT thread_id, session_json, revision FROM tab_session
                     WHERE device_id = ?1 ORDER BY thread_id",
                )?;
                let rows = stmt.query_map([device], |r| {
                    Ok(TabSession {
                        thread_id: ThreadId::new(r.get(0)?),
                        session_json: r.get(1)?,
                        revision: TabRevision(r.get(2)?),
                    })
                })?;
                rows.collect()
            })
            .await
    }

    /// Every device's tabs of every thread, by device then thread: what an
    /// agent reads of the tabs open on the person's devices.
    pub async fn list_all(&self) -> Result<Vec<(DeviceId, TabSession)>, DomainError> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_id, thread_id, session_json, revision FROM tab_session
                     ORDER BY device_id, thread_id",
                )?;
                let rows = stmt.query_map([], |r| {
                    let device: String = r.get(0)?;
                    Ok((
                        device,
                        TabSession {
                            thread_id: ThreadId::new(r.get(1)?),
                            session_json: r.get(2)?,
                            revision: TabRevision(r.get(3)?),
                        },
                    ))
                })?;
                // A device id that doesn't parse can't have been saved (the
                // RPC layer takes it from a caller), so such a row is skipped.
                Ok(rows
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter_map(|(d, s)| DeviceId::parse(&d).ok().map(|d| (d, s)))
                    .collect())
            })
            .await
    }

    /// Keep `session_json` as `device`'s tabs of `thread`, if the row is still
    /// at `revision` (`None`: the device has no row for it yet). When it moved
    /// on since, nothing is written and the answer says where it is.
    pub async fn save(
        &self,
        device: &DeviceId,
        thread: &ThreadId,
        session_json: String,
        revision: Option<TabRevision>,
    ) -> Result<TabSave, DomainError> {
        let revision = revision.map(|r| r.0);
        if session_json.len() > MAX_SESSION_BYTES {
            return Err(DomainError::Invalid(format!(
                "a thread's tabs may be at most {MAX_SESSION_BYTES} bytes, not {}",
                session_json.len()
            )));
        }
        let device = device.as_str().to_string();
        let thread = thread.value();
        let now = Timestamp::now().to_string();
        self.db
            .transaction(move |tx| {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT revision FROM tab_session WHERE device_id = ?1 AND thread_id = ?2",
                        params![device, thread],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        e => Err(e),
                    })
                    .map_err(map_sql_err)?;
                if current != revision {
                    return Ok(TabSave {
                        saved: false,
                        revision: current.map(TabRevision),
                    });
                }
                let next = current.unwrap_or(0) + 1;
                tx.execute(
                    "INSERT INTO tab_session (device_id, thread_id, session_json, revision, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(device_id, thread_id) DO UPDATE SET
                       session_json = excluded.session_json,
                       revision = excluded.revision,
                       updated_at = excluded.updated_at",
                    params![device, thread, session_json, next, now],
                )
                .map_err(map_sql_err)?;
                Ok(TabSave {
                    saved: true,
                    revision: Some(TabRevision(next)),
                })
            })
            .await
    }

    /// Forget `device`'s tabs of `thread`: the thread opens on it as one it
    /// never opened.
    pub async fn forget(&self, device: &DeviceId, thread: &ThreadId) -> Result<(), DomainError> {
        let device = device.as_str().to_string();
        let thread = thread.value();
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM tab_session WHERE device_id = ?1 AND thread_id = ?2",
                    params![device, thread],
                )?;
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

    #[tokio::test]
    async fn a_device_keeps_its_own_tabs_and_a_stale_save_is_refused() {
        let db = Database::in_memory();
        let store = TabSessionStore::new(db.clone());
        let thread = thread(&db).await;
        let (mac, tablet) = (DeviceId::generate(), DeviceId::generate());
        assert!(store.list(&mac).await.unwrap().is_empty());

        let first = store
            .save(&mac, &thread, "{\"a\":1}".into(), None)
            .await
            .unwrap();
        assert_eq!(
            first,
            TabSave {
                saved: true,
                revision: Some(TabRevision(1))
            }
        );
        let second = store
            .save(&mac, &thread, "{\"a\":2}".into(), first.revision)
            .await
            .unwrap();
        assert_eq!(
            second,
            TabSave {
                saved: true,
                revision: Some(TabRevision(2))
            }
        );
        // A late save that read revision 1 changes nothing, and says where the
        // row is now.
        assert_eq!(
            store
                .save(&mac, &thread, "{\"a\":1}".into(), first.revision)
                .await
                .unwrap(),
            TabSave {
                saved: false,
                revision: Some(TabRevision(2))
            }
        );
        assert_eq!(
            store.list(&mac).await.unwrap(),
            vec![TabSession {
                thread_id: thread,
                session_json: "{\"a\":2}".into(),
                revision: TabRevision(2)
            }]
        );
        // Another device has its own.
        assert!(store.list(&tablet).await.unwrap().is_empty());
        store
            .save(&tablet, &thread, "{\"b\":1}".into(), None)
            .await
            .unwrap();
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|(d, s)| d == &tablet && s.session_json == "{\"b\":1}"));
        assert!(all
            .iter()
            .any(|(d, s)| d == &mac && s.session_json == "{\"a\":2}"));
        store.forget(&tablet, &thread).await.unwrap();

        store.forget(&mac, &thread).await.unwrap();
        assert!(store.list(&mac).await.unwrap().is_empty());
        assert_eq!(
            store.save(&mac, &thread, "{}".into(), None).await.unwrap(),
            TabSave {
                saved: true,
                revision: Some(TabRevision(1))
            }
        );
    }

    #[tokio::test]
    async fn a_thread_that_goes_takes_its_tabs() {
        let db = Database::in_memory();
        let store = TabSessionStore::new(db.clone());
        let thread = thread(&db).await;
        let mac = DeviceId::generate();
        store.save(&mac, &thread, "{}".into(), None).await.unwrap();
        crate::SqliteThreadStore::new(db.clone())
            .delete(&thread)
            .await
            .unwrap();
        assert!(store.list(&mac).await.unwrap().is_empty());
    }
}
