//! The requests a commit's changes came from (`commit_request`,
//! `.context/git-integration.md` "A commit's requests"): per path, which
//! requests on the commit's stream changed it since the previous commit that
//! held it, where the file as the work left it is the commit's content.
//!
//! The match needs only paths and content identities. A snapshot row keeps
//! the git blob id of its bytes (`file_snapshot.git_oid`), so a commit's
//! blob is compared with it without reading bytes, and without Local
//! History. The requests come from the turns' files (`v_turn_file`, a time
//! join of a turn with its stream's snapshot line) and each turn's request;
//! never from `request.thread_id`, since a request can span threads and
//! streams.

use rusqlite::{params, Connection, OptionalExtension};

use oxplow_domain::{DomainError, StreamId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// A path a commit changed, with the git blob id it holds (`None`: the commit
/// deleted it).
pub struct CommitPath<'a> {
    pub path: &'a str,
    pub blob: Option<&'a str>,
}

/// Link commit `sha`, committed at `committed` on stream `stream`, to the
/// requests whose changes it holds, per path. Returns how many links it
/// wrote. A path none of whose snapshot rows since the previous commit that
/// held it carries the commit's content links nothing (a commit made
/// outside oxplow, or from a file edited again since).
pub fn link_commit_tx(
    conn: &Connection,
    stream: StreamId,
    sha: &str,
    committed: Timestamp,
    paths: &[CommitPath<'_>],
    at: Timestamp,
) -> Result<usize, DomainError> {
    let committed_text = ts_to_string(committed);
    let mut written = 0;
    for p in paths {
        // The previous commit that held the path: the work since is this
        // commit's.
        let since: String = conn
            .query_row(
                "SELECT max(c.committed_at) FROM git_commit c
                   JOIN git_commit_file f ON f.sha = c.sha
                  WHERE f.path = ?1 AND c.sha <> ?2 AND c.committed_at <= ?3",
                params![p.path, sha, committed_text],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(map_sql_err)?
            .unwrap_or_else(|| "0000".to_string());
        // The file as the work left it is the commit's content: some row
        // since then holds exactly it.
        let holds: bool = match p.blob {
            Some(blob) => conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM file_snapshot
                                 WHERE stream_id = ?1 AND path = ?2 AND captured_at > ?3
                                   AND git_oid = ?4)",
                params![stream.value(), p.path, since, blob],
                |r| r.get(0),
            ),
            None => conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM file_snapshot
                                 WHERE stream_id = ?1 AND path = ?2 AND captured_at > ?3
                                   AND storage = 'deleted')",
                params![stream.value(), p.path, since],
                |r| r.get(0),
            ),
        }
        .map_err(map_sql_err)?;
        if !holds {
            continue;
        }
        // The turns that changed it since, and their requests: a call that
        // named it (or the harness reported) is the stronger basis.
        let mut stmt = conn
            .prepare(
                "SELECT t.request_id, tf.turn_id,
                        CASE tf.claim WHEN 'named' THEN 'named' WHEN 'reported' THEN 'reported'
                                      ELSE 'observed' END
                   FROM v_turn_file tf JOIN agent_turn t ON t.id = tf.turn_id
                  WHERE tf.stream_id = ?1 AND tf.path = ?2 AND t.request_id IS NOT NULL
                    AND tf.last_changed_at > ?3
                    AND tf.first_changed_at <= strftime('%Y-%m-%dT%H:%M:%fZ',
                                                        julianday(?4) + 60.0 / 86400)
                  ORDER BY t.request_id, tf.turn_id",
            )
            .map_err(map_sql_err)?;
        let found: Vec<(i64, i64, String)> = stmt
            .query_map(
                params![stream.value(), p.path, since, committed_text],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(map_sql_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_sql_err)?;
        let mut requests: Vec<(i64, i64, String)> = Vec::new();
        for (request, turn, basis) in found {
            match requests.iter_mut().find(|(r, _, _)| *r == request) {
                Some(existing) => {
                    if rank(&basis) > rank(&existing.2) {
                        existing.1 = turn;
                        existing.2 = basis;
                    }
                }
                None => requests.push((request, turn, basis)),
            }
        }
        let confidence = if requests.len() == 1 {
            "sole"
        } else {
            "shared"
        };
        for (request, turn, basis) in requests {
            conn.execute(
                "INSERT OR REPLACE INTO commit_request
                   (sha, path, request_id, basis, confidence, turn_id, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    sha,
                    p.path,
                    request,
                    basis,
                    confidence,
                    turn,
                    ts_to_string(at)
                ],
            )
            .map_err(map_sql_err)?;
            written += 1;
        }
    }
    Ok(written)
}

/// How strong a basis is: a call that named the path, the harness's report,
/// then only that it changed while the request ran.
fn rank(basis: &str) -> u8 {
    match basis {
        "named" => 2,
        "reported" => 1,
        _ => 0,
    }
}

/// The stream whose worktree is `worktree`, if any.
pub fn stream_of_worktree_tx(
    conn: &Connection,
    worktree: &str,
) -> Result<Option<StreamId>, DomainError> {
    conn.query_row(
        "SELECT id FROM streams WHERE worktree_path = ?1 ORDER BY id LIMIT 1",
        params![worktree],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|id| id.map(StreamId::new))
    .map_err(map_sql_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// A stream with one thread, and the helpers the cases share.
    struct Rig {
        db: Database,
    }

    impl Rig {
        async fn new() -> Self {
            let db = Database::in_memory();
            db.transaction(|tx| {
                tx.execute_batch(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                       VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/w', 'a', 'a');
                     INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (1, 1, 'A', 'a', 'a');
                     INSERT INTO threads (id, stream_id, title, created_at, updated_at) VALUES (2, 1, 'B', 'a', 'a');",
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
            Rig { db }
        }

        /// A turn (and its own request) on `thread` between minutes `from`
        /// and `to` of 2026-01-01, which captured `path` as `oid` at minute
        /// `at` (a named claim when `claim`).
        async fn turn(
            &self,
            id: i64,
            thread: i64,
            minutes: (u32, u32, u32),
            path: &'static str,
            oid: &'static str,
            claim: bool,
        ) {
            let (from, to, at) = minutes;
            let t = |m: u32| format!("2026-01-01T00:{m:02}:00.000000Z");
            self.db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO request (id, thread_id) VALUES (?1, ?2)",
                        params![id, thread],
                    )
                    .map_err(map_sql_err)?;
                    tx.execute(
                        "INSERT INTO agent_turn (id, thread_id, prompt, started_at, ended_at, request_id)
                         VALUES (?1, ?2, 'p', ?3, ?4, ?1)",
                        params![id, thread, t(from), t(to)],
                    )
                    .map_err(map_sql_err)?;
                    // A take, logged as the store does: its file, then its op.
                    tx.execute(
                        "INSERT INTO snapshot (stream_id, created_at) VALUES (1, ?1)",
                        params![t(at)],
                    )
                    .map_err(map_sql_err)?;
                    let snapshot = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO file_snapshot (stream_id, path, blob_hash, captured_at, storage, git_oid,
                                                    first_changed_at, last_changed_at, snapshot_id)
                         VALUES (1, ?1, ?2, ?3, 'git', ?2, ?3, ?3, ?4)",
                        params![path, oid, t(at), snapshot],
                    )
                    .map_err(map_sql_err)?;
                    tx.execute(
                        "INSERT INTO snapshot_op (stream_id, snapshot_id, trigger, at, elapsed_ms, file_count)
                         VALUES (1, ?1, 'quiet', ?2, 0, 1)",
                        params![snapshot, t(at)],
                    )
                    .map_err(map_sql_err)?;
                    if claim {
                        tx.execute(
                            "INSERT INTO agent_tool_call (thread_id, turn_id, tool, at, kind)
                             VALUES (?1, ?2, 'Edit', ?3, 'edit')",
                            params![thread, id, t(at)],
                        )
                        .map_err(map_sql_err)?;
                        let call = tx.last_insert_rowid();
                        tx.execute(
                            "INSERT INTO agent_tool_call_path (call_id, path, basis) VALUES (?1, ?2, 'named')",
                            params![call, path],
                        )
                        .map_err(map_sql_err)?;
                    }
                    Ok(())
                })
                .await
                .unwrap();
        }

        /// Commit `sha` at minute `m` holding `paths`, linked.
        async fn commit(
            &self,
            sha: &'static str,
            m: u32,
            paths: &'static [(&'static str, &'static str)],
        ) -> usize {
            let committed = Timestamp::parse(&format!("2026-01-01T00:{m:02}:30.000000Z")).unwrap();
            self.db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO git_commit (sha, author, email, committed_at, subject)
                         VALUES (?1, 'a', 'a@x', ?2, 's')",
                        params![sha, ts_to_string(committed)],
                    )
                    .map_err(map_sql_err)?;
                    for (path, _) in paths {
                        tx.execute(
                            "INSERT INTO git_commit_file (sha, path, status) VALUES (?1, ?2, 'M')",
                            params![sha, path],
                        )
                        .map_err(map_sql_err)?;
                    }
                    let held: Vec<CommitPath<'_>> = paths
                        .iter()
                        .map(|(path, blob)| CommitPath {
                            path,
                            blob: Some(blob),
                        })
                        .collect();
                    link_commit_tx(tx, StreamId::new(1), sha, committed, &held, committed)
                })
                .await
                .unwrap()
        }

        async fn links(&self) -> Vec<(String, String, i64, String, String)> {
            self.db
                .read(|c| {
                    let mut stmt = c
                        .prepare(
                            "SELECT sha, path, request_id, basis, confidence FROM commit_request
                              ORDER BY sha, path, request_id",
                        )
                        .map_err(map_sql_err)?;
                    let rows = stmt
                        .query_map([], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                        })
                        .map_err(map_sql_err)?;
                    rows.collect::<Result<Vec<_>, _>>().map_err(map_sql_err)
                })
                .await
                .unwrap()
        }
    }

    fn link(
        sha: &str,
        path: &str,
        request: i64,
        basis: &str,
        confidence: &str,
    ) -> (String, String, i64, String, String) {
        (
            sha.into(),
            path.into(),
            request,
            basis.into(),
            confidence.into(),
        )
    }

    /// A commit of one request's file links to it; one of two requests' files
    /// links each per path; a commit made outside oxplow, or of a file that
    /// was edited again, links nothing.
    #[tokio::test]
    async fn a_commit_links_the_requests_whose_files_it_holds() {
        let rig = Rig::new().await;
        rig.turn(1, 1, (1, 3, 2), "a.rs", "oidA1", true).await;
        rig.turn(2, 2, (4, 6, 5), "b.rs", "oidB1", false).await;
        let n = rig
            .commit(
                "c1",
                8,
                &[("a.rs", "oidA1"), ("b.rs", "oidB1"), ("c.rs", "outside")],
            )
            .await;
        assert_eq!(n, 2);
        assert_eq!(
            rig.links().await,
            vec![
                link("c1", "a.rs", 1, "named", "sole"),
                link("c1", "b.rs", 2, "observed", "sole"),
            ]
        );
    }

    /// Two threads' requests that changed one file are both linked, marked
    /// shared; a file changed again by a later request, after a commit that
    /// held it, links to the later one only.
    #[tokio::test]
    async fn two_requests_on_one_file_are_shared_and_a_later_one_is_alone() {
        let rig = Rig::new().await;
        rig.turn(1, 1, (1, 3, 2), "f.rs", "oid1", false).await;
        rig.turn(2, 2, (2, 4, 3), "f.rs", "oid2", false).await;
        rig.commit("c1", 5, &[("f.rs", "oid2")]).await;
        rig.turn(3, 1, (6, 8, 7), "f.rs", "oid3", false).await;
        rig.commit("c2", 9, &[("f.rs", "oid3")]).await;
        assert_eq!(
            rig.links().await,
            vec![
                link("c1", "f.rs", 1, "observed", "shared"),
                link("c1", "f.rs", 2, "observed", "shared"),
                link("c2", "f.rs", 3, "observed", "sole"),
            ]
        );
    }
}
