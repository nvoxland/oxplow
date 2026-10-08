//! Observed files (`.context/work-tracking.md` "Attribution is core's"):
//! the files a thread's turn changed belong to the effort that holds the
//! turn. Most edits go through a shell, which names no file, so a file an
//! edit tool claimed is `claimed` and every other file the turn changed is
//! `observed` — read from the turn's own snapshot bracket on its
//! `thread.checkpoint`, after the effort policy has had its say (it may
//! have just opened the effort that adopts the turn).
//!
//! A file another thread's effort claimed during the turn is that
//! thread's. A file two threads changed at once and neither claimed is
//! observed by both: it changed during each, and `source` says nobody
//! claimed it. Changes between turns (the person's own) belong to no one.

use async_trait::async_trait;
use oxplow_db::effort_store::{EffortFileChange, EffortStore as _, OwnedFileRefVersion};
use oxplow_db::{SqlCell, SqliteEffortStore, SqliteSnapshotStore};
use oxplow_domain::events::schema::{EventType as _, ThreadCheckpoint, ThreadCheckpointV1};
use oxplow_domain::tree_diff::ChangeStatus;
use oxplow_domain::{DomainError, EffortId, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::sql_gateway::SqlGateway;

/// The consumer's name (its checkpoint key; what callers settle on).
pub const NAME: &str = "effort.observe";

pub struct EffortObservationConsumer {
    pub efforts: std::sync::Arc<SqliteEffortStore>,
    pub snapshots: std::sync::Arc<SqliteSnapshotStore>,
    pub sql: SqlGateway,
    /// Resolves the VCS revision a file was at, like a claim's.
    pub lifecycle: crate::effort_service::EffortService,
}

impl EffortObservationConsumer {
    /// The turn's bracket and window: (start snapshot, started_at, ended_at).
    async fn turn(&self, turn: i64) -> Result<Option<(Option<i64>, String, String)>, DomainError> {
        let rows = self
            .sql
            .query_sql(
                "SELECT start_snapshot_id, started_at, ended_at FROM v_agent_turn WHERE id = ?1",
                vec![SqlCell::Int(turn)],
                None,
            )
            .await?
            .rows;
        Ok(rows.into_iter().next().and_then(|row| match &row[..] {
            [start, SqlCell::Text(from), SqlCell::Text(to)] => Some((
                match start {
                    SqlCell::Int(s) => Some(*s),
                    _ => None,
                },
                from.clone(),
                to.clone(),
            )),
            _ => None,
        }))
    }
}

/// The effort holding `thread`'s work at `at`.
async fn effort_at(
    sql: &SqlGateway,
    thread: i64,
    at: &str,
) -> Result<Option<EffortId>, DomainError> {
    let rows = sql
        .query_sql(
            "SELECT id FROM v_effort
              WHERE thread_id = ?1 AND started_at <= ?2
                AND (ended_at IS NULL OR ended_at >= ?2)
              ORDER BY started_at DESC LIMIT 1",
            vec![SqlCell::Int(thread), SqlCell::Text(at.into())],
            None,
        )
        .await?
        .rows;
    Ok(match rows.first().and_then(|r| r.first()) {
        Some(SqlCell::Int(id)) => Some(EffortId::new(*id)),
        _ => None,
    })
}

/// The effort holding `turn`'s end: the one its files and hints belong to.
pub(crate) async fn effort_at_turn_end(
    sql: &SqlGateway,
    turn: i64,
) -> Result<Option<EffortId>, DomainError> {
    let rows = sql
        .query_sql(
            "SELECT thread_id, ended_at FROM v_agent_turn WHERE id = ?1",
            vec![SqlCell::Int(turn)],
            None,
        )
        .await?
        .rows;
    match rows.first().map(|r| &r[..]) {
        Some([SqlCell::Int(thread), SqlCell::Text(to)]) => effort_at(sql, *thread, to).await,
        _ => Ok(None),
    }
}

#[async_trait]
impl AsyncEventConsumer for EffortObservationConsumer {
    fn name(&self) -> &'static str {
        NAME
    }

    /// The policy first: it may open the effort that adopts the turn.
    fn after(&self) -> Vec<String> {
        vec![crate::effort_policy::NAME.to_string()]
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == ThreadCheckpoint::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let checkpoint: ThreadCheckpointV1 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("thread.checkpoint: {e}")))?;
        let (Some(thread), Some(turn), Some(end)) = (
            event.envelope.anchors.thread_id,
            event.envelope.anchors.turn_id,
            event.envelope.anchors.snapshot_id,
        ) else {
            return Ok(());
        };
        if !checkpoint.changed {
            return Ok(());
        }
        let Some((start, from, to)) = self.turn(turn).await? else {
            return Ok(());
        };
        let Some(effort) = effort_at(&self.sql, thread.value(), &to).await? else {
            return Ok(());
        };
        // From the later of the turn's start and the effort's: an effort
        // that began mid-turn (after a commit closed the last) holds only
        // what came after. Its start pin may not be taken yet (the policy
        // just opened it): take it now (idempotent).
        self.lifecycle.on_effort_opened(effort).await?;
        let effort_start = self
            .efforts
            .get_effort(&effort)
            .await?
            .and_then(|e| e.start_snapshot_id);
        let start = match (start, effort_start) {
            (Some(t), Some(e)) => Some(t.max(e)),
            (t, e) => t.or(e),
        };
        let changes: Vec<(String, EffortFileChange)> = self
            .snapshots
            .diff_snapshots(start, end)
            .await?
            .into_iter()
            .map(|c| {
                let change = match c.status {
                    ChangeStatus::Added => EffortFileChange::Created,
                    ChangeStatus::Modified => EffortFileChange::Updated,
                    ChangeStatus::Deleted => EffortFileChange::Deleted,
                };
                (c.path, change)
            })
            .collect();
        let claimable: Vec<String> = changes.iter().map(|(p, _)| p.clone()).collect();
        let claimable = self.lifecycle.claimable_paths(&thread, &claimable).await;
        let changes = changes
            .into_iter()
            .filter(|(p, _)| claimable.contains(p))
            .collect();
        let v = self.lifecycle.file_version_at(&thread, end).await;
        self.efforts
            .observe_files(
                &effort,
                thread,
                (from, to),
                changes,
                OwnedFileRefVersion {
                    local_snapshot_id: v.local_snapshot_id,
                    closest_vcs_rev: v.closest_vcs_rev,
                    vcs_rev_exact: v.vcs_rev_exact,
                },
            )
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::EffortFixture;
    use crate::thread_checkpoint::tests::{turn, with_baseline};

    async fn settle(f: &EffortFixture) {
        f.svc
            .event_pump
            .settle(
                &[crate::effort_policy::NAME, NAME],
                std::time::Duration::from_secs(10),
            )
            .await;
    }

    /// (path, source) of an effort's files, by path.
    async fn files(f: &EffortFixture, effort: EffortId) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = f
            .svc
            .effort_store
            .list_files(&effort)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.path, e.source.as_str().to_string()))
            .collect();
        out.sort();
        out
    }

    /// A turn's changed files are its effort's: `observed`, unless an edit
    /// tool claimed one; a later turn that changed nothing adds nothing.
    #[tokio::test]
    async fn a_turns_changed_files_are_its_efforts() {
        let f = with_baseline().await;
        f.svc
            .effort_store
            .record_file(
                &f.effort,
                "made.txt",
                oxplow_db::effort_store::EffortFileChange::Created,
                oxplow_db::effort_store::FileRefVersion {
                    local_snapshot_id: 0,
                    closest_vcs_rev: None,
                    vcs_rev_exact: false,
                },
            )
            .await
            .unwrap();
        turn(&f, Some(("made.txt", "x")), &["Edit"]).await;
        turn(&f, Some(("shell.txt", "y")), &["Bash"]).await;
        settle(&f).await;
        assert_eq!(
            files(&f, f.effort).await,
            vec![
                ("made.txt".into(), "claimed".into()),
                ("shell.txt".into(), "observed".into()),
            ]
        );
    }

    /// The effort rule 2 opens for a turn holds that turn's files.
    #[tokio::test]
    async fn an_effort_opened_for_a_turn_holds_its_files() {
        let f = with_baseline().await;
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::effort::CLOSE,
                serde_json::json!({ "effort": oxplow_domain::refs::build::effort_ref(f.effort) }),
                false,
            )
            .await
            .unwrap();
        turn(&f, Some(("shell.txt", "y")), &["Bash"]).await;
        settle(&f).await;
        let opened = f
            .svc
            .effort_store
            .find_open_for_thread(&f.thread)
            .await
            .unwrap()
            .expect("rule 2 opened one");
        assert_eq!(
            files(&f, opened.id).await,
            vec![("shell.txt".into(), "observed".into())]
        );
    }
}
