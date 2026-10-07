//! Persisted agent nudges: the informational steers oxplow surfaces to the
//! agent from the PostToolUse hook (report-less-test-run, coverage-target).
//! Previously fully ephemeral — see migration `V33__agent_nudge.sql` and
//! `.context/agent-model.md` (Nudge persistence).
//!
//! A thin typed read/write surface. The one-shot
//! dedup (so a nudge fires at most once per effort) lives in the
//! service, so the store only ever records nudges that actually fired.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, ThreadId, Timestamp};

use crate::database::Database;
use crate::database::{string_to_ts, ts_to_string};

/// Who a nudge is for: the coding agent (taken by its next hook) or a
/// person (raised in Alerts until they dismiss it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    #[default]
    Agent,
    Person,
}

impl Audience {
    pub fn as_str(self) -> &'static str {
        match self {
            Audience::Agent => "agent",
            Audience::Person => "person",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "person" {
            Audience::Person
        } else {
            Audience::Agent
        }
    }
}

/// One persisted nudge row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AgentNudge {
    pub id: i64,
    pub thread_id: String,
    /// Open effort the nudge fired against, if any (some nudge kinds fire
    /// thread-scoped with no open effort).
    pub effort_id: Option<String>,
    /// Well-known kind: `report-less-run` | `coverage-target` (open-ended).
    pub kind: String,
    /// The full message text that was surfaced to the agent.
    pub message: String,
    /// What caused it — the bash command (or commit sha).
    pub trigger: Option<String>,
    pub created_at: Timestamp,
    /// The turn it fired in (`agent_turn.id`), when known.
    pub turn_id: Option<i64>,
    /// When a hook response carried it to the agent, or the person
    /// dismissed it; `None` until then.
    pub delivered_at: Option<Timestamp>,
    pub audience: Audience,
}

/// Write-side input — `id` and `created_at` are assigned by the store.
#[derive(Debug, Clone, Default)]
pub struct NewAgentNudge {
    pub thread_id: String,
    pub effort_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub trigger: Option<String>,
    pub turn_id: Option<i64>,
    /// The event that fired it; a second nudge of the same kind for the
    /// same cause (a redelivered event) is not written.
    pub cause: Option<String>,
    pub audience: Audience,
}

fn row_to_nudge(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentNudge> {
    let created_at: String = row.get(6)?;
    let delivered_at: Option<String> = row.get(8)?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(AgentNudge {
        id: row.get(0)?,
        thread_id: ThreadId::new(row.get::<_, i64>(1)?).to_string(),
        effort_id: row
            .get::<_, Option<i64>>(2)?
            .map(|v| EffortId::new(v).to_string()),
        kind: row.get(3)?,
        message: row.get(4)?,
        trigger: row.get(5)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        turn_id: row.get(7)?,
        delivered_at: delivered_at
            .as_deref()
            .map(string_to_ts)
            .transpose()
            .map_err(map_err)?,
        audience: Audience::parse(&row.get::<_, String>(9)?),
    })
}

const SELECT_COLS: &str =
    "id, thread_id, effort_id, kind, message, trigger, created_at, turn_id, delivered_at, audience";

/// Where a one-shot mark is kept: a thread, or one of its efforts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnceScope {
    pub thread: i64,
    pub effort: Option<i64>,
}

impl OnceScope {
    pub fn thread(thread: i64) -> Self {
        Self {
            thread,
            effort: None,
        }
    }

    pub fn effort(thread: i64, effort: i64) -> Self {
        Self {
            thread,
            effort: Some(effort),
        }
    }
}

#[derive(Clone)]
pub struct SqliteAgentNudgeStore {
    db: Database,
}

impl SqliteAgentNudgeStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Insert a nudge row. Returns the new row id, or `None` when a nudge
    /// of this kind was already recorded for the same cause.
    pub async fn record(&self, nudge: NewAgentNudge) -> Result<Option<i64>, DomainError> {
        self.db.transaction(move |tx| record_tx(tx, &nudge)).await
    }

    /// The agent's undelivered nudges on the thread, oxplow's own before
    /// advisories (`<extension>/<id>` kinds) and oldest first within each,
    /// up to `budget` characters of message (always at least one), marked
    /// delivered in the same transaction — so each reaches the agent once,
    /// on whichever of the thread's hooks comes first, and what the budget
    /// held waits for the next.
    pub async fn take_for_agent(
        &self,
        thread_id: &str,
        budget: usize,
    ) -> Result<Vec<AgentNudge>, DomainError> {
        let thread_val = ThreadId::try_from_str(thread_id)
            .ok_or_else(|| DomainError::Invalid(format!("bad thread id: {thread_id}")))?
            .value();
        self.db
            .transaction(move |tx| {
                let sql_err = crate::database::map_sql_err;
                let mut waiting = tx
                    .prepare(&format!(
                        "SELECT {SELECT_COLS} FROM agent_nudge
                          WHERE thread_id = ?1 AND delivered_at IS NULL AND audience = 'agent'
                          ORDER BY instr(kind, '/') > 0, id"
                    ))
                    .map_err(sql_err)?
                    .query_map(params![thread_val], row_to_nudge)
                    .map_err(sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(sql_err)?;
                let mut used = 0;
                let mut taken = 0;
                for n in &waiting {
                    let len = n.message.chars().count();
                    if taken > 0 && used + len > budget {
                        break;
                    }
                    used += len;
                    taken += 1;
                }
                waiting.truncate(taken);
                let now = Timestamp::now();
                for n in &mut waiting {
                    tx.execute(
                        "UPDATE agent_nudge SET delivered_at = ?2 WHERE id = ?1",
                        params![n.id, ts_to_string(now)],
                    )
                    .map_err(sql_err)?;
                    n.delivered_at = Some(now);
                }
                Ok(waiting)
            })
            .await
    }

    /// How many `kind` nudges reached the agent in `scope`: within the
    /// effort, or anywhere on the thread.
    pub async fn delivered(&self, scope: OnceScope, kind: &str) -> Result<i64, DomainError> {
        let kind = kind.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT count(*) FROM agent_nudge
                      WHERE thread_id = ?1 AND (?2 IS NULL OR effort_id = ?2) AND kind = ?3
                        AND audience = 'agent' AND delivered_at IS NOT NULL",
                    params![scope.thread, scope.effort, kind],
                    |r| r.get(0),
                )
            })
            .await
    }

    /// Count one evaluation of each of `hints` on `thread`.
    pub async fn evaluated(&self, thread: i64, hints: Vec<String>) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                let now = ts_to_string(Timestamp::now());
                for hint in &hints {
                    tx.execute(
                        "INSERT INTO hint_stat (thread_id, hint, evaluated, last_evaluated_at)
                         VALUES (?1, ?2, 1, ?3)
                         ON CONFLICT (thread_id, hint)
                         DO UPDATE SET evaluated = evaluated + 1, last_evaluated_at = ?3",
                        params![thread, hint, now],
                    )
                    .map_err(crate::database::map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Claim the one-shot `mark` in `scope` (`report-less-run`,
    /// `<extension>/<advisory>`, …). `true` the first time, `false` once
    /// it has fired — durably, across restarts.
    pub async fn claim_once(&self, scope: OnceScope, mark: &str) -> Result<bool, DomainError> {
        let mark = mark.to_string();
        self.db
            .transaction(move |tx| claim_once_tx(tx, scope, &mark))
            .await
    }

    /// Whether `mark` has fired in `scope`.
    pub async fn has_fired(&self, scope: OnceScope, mark: &str) -> Result<bool, DomainError> {
        let mark = mark.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM once_mark
                      WHERE thread_id = ?1 AND coalesce(effort_id, 0) = coalesce(?2, 0)
                        AND mark = ?3)",
                    params![scope.thread, scope.effort, mark],
                    |r| r.get(0),
                )
            })
            .await
    }

    /// Nudges fired against an effort, newest-first.
    pub async fn list_for_effort(&self, effort_id: &str) -> Result<Vec<AgentNudge>, DomainError> {
        let effort_val = EffortId::try_from_str(effort_id).map(|e| e.value());
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM agent_nudge
                      WHERE effort_id = ?1
                      ORDER BY created_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![effort_val], row_to_nudge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// All nudges for a thread (effort-scoped and thread-only), newest-first.
    pub async fn list_for_thread(&self, thread_id: &str) -> Result<Vec<AgentNudge>, DomainError> {
        let thread_val = ThreadId::try_from_str(thread_id).map(|t| t.value());
        self.db
            .call(move |conn| {
                let sql = format!(
                    "SELECT {SELECT_COLS} FROM agent_nudge
                      WHERE thread_id = ?1
                      ORDER BY created_at DESC, id DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread_val], row_to_nudge)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

/// Write `nudge` in the caller's transaction; `None` when this kind was
/// already recorded for its cause.
pub fn record_tx(
    conn: &rusqlite::Connection,
    nudge: &NewAgentNudge,
) -> Result<Option<i64>, DomainError> {
    let sql_err = crate::database::map_sql_err;
    let thread_val = ThreadId::try_from_str(&nudge.thread_id)
        .ok_or_else(|| DomainError::Invalid(format!("bad thread id: {}", nudge.thread_id)))?
        .value();
    let effort_val = match &nudge.effort_id {
        Some(e) => Some(
            EffortId::try_from_str(e)
                .ok_or_else(|| DomainError::Invalid(format!("bad effort id: {e}")))?
                .value(),
        ),
        None => None,
    };
    let now = ts_to_string(Timestamp::now());
    conn.query_row(
        "INSERT INTO agent_nudge
           (thread_id, effort_id, kind, message, trigger, created_at, turn_id, cause, audience)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (cause, kind, coalesce(effort_id, 0)) WHERE cause IS NOT NULL DO NOTHING
         RETURNING id",
        params![
            thread_val,
            effort_val,
            nudge.kind,
            nudge.message,
            nudge.trigger,
            now,
            nudge.turn_id,
            nudge.cause,
            nudge.audience.as_str(),
        ],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

/// A person dismisses a hint raised to them: `true` when it was theirs and
/// still raised.
pub fn dismiss_tx(conn: &rusqlite::Connection, id: i64) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            "UPDATE agent_nudge SET delivered_at = ?2
              WHERE id = ?1 AND audience = 'person' AND delivered_at IS NULL",
            params![id, ts_to_string(Timestamp::now())],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

/// Claim `mark` in `scope` in the caller's transaction: `true` the first
/// time, `false` when it has already fired.
pub fn claim_once_tx(
    conn: &rusqlite::Connection,
    scope: OnceScope,
    mark: &str,
) -> Result<bool, DomainError> {
    let n = conn
        .execute(
            "INSERT INTO once_mark (thread_id, effort_id, mark, fired_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (thread_id, coalesce(effort_id, 0), mark) DO NOTHING",
            params![
                scope.thread,
                scope.effort,
                mark,
                ts_to_string(Timestamp::now())
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal stream → thread → task → effort chain so FK-on
    /// inserts of nudges succeed. Returns `(store, thread_id, effort_id)`.
    async fn fixture() -> (SqliteAgentNudgeStore, String, String) {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-06-13T00:00:00Z";
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                     VALUES (1, 1, 't', 'active', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES (1, 't', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                let task_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                     VALUES ('work_item:oxplow:tsk' || ?1, 1, ?2)",
                    params![task_id, now],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        (
            SqliteAgentNudgeStore::new(db),
            "thr1".to_string(),
            "eff1".to_string(),
        )
    }

    fn sample(kind: &str, effort: Option<&str>) -> NewAgentNudge {
        NewAgentNudge {
            thread_id: "thr1".into(),
            effort_id: effort.map(str::to_string),
            kind: kind.into(),
            message: format!("a {kind} nudge"),
            trigger: Some("cargo test".into()),
            ..Default::default()
        }
    }

    /// P3.2 (tsk472): a nudge reaches the agent exactly once, whichever hook
    /// response picks it up; a redelivered cause fires nothing new.
    #[tokio::test]
    async fn undelivered_nudges_are_taken_once_and_causes_dedupe() {
        let (store, thread, _effort) = fixture().await;
        let caused = |kind: &str| NewAgentNudge {
            cause: Some("evt-1".into()),
            ..sample(kind, Some("eff1"))
        };
        assert!(store
            .record(caused("report-less-run"))
            .await
            .unwrap()
            .is_some());
        assert!(store
            .record(caused("report-less-run"))
            .await
            .unwrap()
            .is_none());
        assert!(store
            .record(caused("coverage-target"))
            .await
            .unwrap()
            .is_some());
        // The same event and kind for another effort (here: none) is its
        // own nudge (tsk510).
        assert!(store
            .record(NewAgentNudge {
                cause: Some("evt-1".into()),
                ..sample("report-less-run", None)
            })
            .await
            .unwrap()
            .is_some());
        let taken = store.take_for_agent(&thread, usize::MAX).await.unwrap();
        assert_eq!(
            taken.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(),
            vec!["report-less-run", "coverage-target", "report-less-run"],
            "oldest first"
        );
        assert!(taken.iter().all(|n| n.delivered_at.is_some()));
        assert!(store
            .take_for_agent(&thread, usize::MAX)
            .await
            .unwrap()
            .is_empty());
    }

    /// A one-shot mark is durable: a new store over the same database (a
    /// restart) sees it, and claiming it again says so. A thread's mark and
    /// its effort's are separate.
    #[tokio::test]
    async fn once_marks_survive_a_restart() {
        let (store, _thread, _effort) = fixture().await;
        let effort = OnceScope::effort(1, 1);
        let thread = OnceScope::thread(1);
        assert!(store.claim_once(effort, "report-less-run").await.unwrap());
        assert!(!store.claim_once(effort, "report-less-run").await.unwrap());
        assert!(!store.has_fired(thread, "report-less-run").await.unwrap());
        assert!(store.claim_once(thread, "report-less-run").await.unwrap());
        let restarted = SqliteAgentNudgeStore::new(store.db.clone());
        assert!(restarted
            .has_fired(effort, "report-less-run")
            .await
            .unwrap());
        assert!(restarted
            .has_fired(thread, "report-less-run")
            .await
            .unwrap());
        assert!(!restarted.has_fired(effort, "acme/other").await.unwrap());
    }

    /// The agent takes its undelivered nudges oldest first, oxplow's own
    /// before advisories, up to a character budget (always at least one);
    /// the rest are held for its next hook. A person's are never its.
    #[tokio::test]
    async fn the_agent_takes_its_nudges_within_a_budget() {
        let (store, thread, _effort) = fixture().await;
        let nudge = |kind: &str, message: &str, audience: Audience| NewAgentNudge {
            thread_id: "thr1".into(),
            kind: kind.into(),
            message: message.into(),
            audience,
            ..Default::default()
        };
        store
            .record(nudge("x/a", &"a".repeat(30), Audience::Agent))
            .await
            .unwrap();
        store
            .record(nudge("x/p", "for you", Audience::Person))
            .await
            .unwrap();
        store
            .record(nudge("report-less-run", &"r".repeat(10), Audience::Agent))
            .await
            .unwrap();
        store
            .record(nudge("x/b", &"b".repeat(30), Audience::Agent))
            .await
            .unwrap();
        let kinds = |ns: Vec<AgentNudge>| ns.into_iter().map(|n| n.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds(store.take_for_agent(&thread, 45).await.unwrap()),
            vec!["report-less-run", "x/a"]
        );
        assert_eq!(
            kinds(store.take_for_agent(&thread, 5).await.unwrap()),
            vec!["x/b"],
            "at least one"
        );
        assert!(store.take_for_agent(&thread, 45).await.unwrap().is_empty());
    }

    /// A person dismisses a hint raised to them, once; an agent's nudge
    /// isn't theirs to dismiss.
    #[tokio::test]
    async fn a_person_dismisses_their_hint() {
        let (store, _thread, _effort) = fixture().await;
        let raise = |audience: Audience| NewAgentNudge {
            thread_id: "thr1".into(),
            kind: "x/p".into(),
            message: "m".into(),
            audience,
            ..Default::default()
        };
        let mine = store
            .record(raise(Audience::Person))
            .await
            .unwrap()
            .unwrap();
        let agents = store.record(raise(Audience::Agent)).await.unwrap().unwrap();
        let dismiss = |id: i64| store.db.transaction(move |tx| dismiss_tx(tx, id));
        assert!(dismiss(mine).await.unwrap());
        assert!(!dismiss(mine).await.unwrap(), "once");
        assert!(!dismiss(agents).await.unwrap(), "not a person's");
    }

    /// How often a kind reached the agent in a scope, and how often each
    /// hint was evaluated on a thread.
    #[tokio::test]
    async fn deliveries_and_evaluations_are_counted() {
        let (store, thread, _effort) = fixture().await;
        for effort in [Some("eff1"), Some("eff1"), None] {
            store
                .record(NewAgentNudge {
                    thread_id: "thr1".into(),
                    effort_id: effort.map(str::to_string),
                    kind: "x/a".into(),
                    message: "m".into(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        assert_eq!(
            store
                .delivered(OnceScope::effort(1, 1), "x/a")
                .await
                .unwrap(),
            0
        );
        store.take_for_agent(&thread, 1_000).await.unwrap();
        assert_eq!(
            store
                .delivered(OnceScope::effort(1, 1), "x/a")
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            store.delivered(OnceScope::thread(1), "x/a").await.unwrap(),
            3
        );
        store
            .evaluated(1, vec!["x/a".into(), "x/b".into()])
            .await
            .unwrap();
        store.evaluated(1, vec!["x/a".into()]).await.unwrap();
        let counts: Vec<(String, i64)> = store
            .db
            .call(|c| {
                c.prepare("SELECT hint, evaluated FROM hint_stat ORDER BY hint")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .await
            .unwrap();
        assert_eq!(counts, vec![("x/a".into(), 2), ("x/b".into(), 1)]);
    }

    #[tokio::test]
    async fn record_then_list_round_trips_fields() {
        let (store, _thread, effort) = fixture().await;
        let id = store
            .record(sample("report-less-run", Some("eff1")))
            .await
            .unwrap()
            .expect("a new nudge is written");
        let got = store.list_for_effort(&effort).await.unwrap();
        assert_eq!(got.len(), 1);
        let n = &got[0];
        assert_eq!(n.id, id);
        assert_eq!(n.kind, "report-less-run");
        assert_eq!(n.effort_id.as_deref(), Some("eff1"));
        assert_eq!(n.thread_id, "thr1");
        assert_eq!(n.message, "a report-less-run nudge");
        assert_eq!(n.trigger.as_deref(), Some("cargo test"));
    }

    #[tokio::test]
    async fn list_for_thread_includes_effort_scoped_and_thread_only() {
        let (store, thread, _effort) = fixture().await;
        store
            .record(sample("report-less-run", Some("eff1")))
            .await
            .unwrap();
        // A thread-only nudge (no open effort).
        store.record(sample("configure", None)).await.unwrap();
        let all = store.list_for_thread(&thread).await.unwrap();
        assert_eq!(all.len(), 2);
        // The effort filter only sees the effort-scoped one.
        let eff = store.list_for_effort("eff1").await.unwrap();
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].kind, "report-less-run");
    }

    #[tokio::test]
    async fn deleting_effort_cascades_to_nudges() {
        let (store, thread, effort) = fixture().await;
        store
            .record(sample("coverage-target", Some("eff1")))
            .await
            .unwrap();
        // Deleting the parent effort removes its nudges (ON DELETE CASCADE).
        store
            .db
            .call(|conn| conn.execute("DELETE FROM effort WHERE id = 1", []))
            .await
            .unwrap();
        assert!(store.list_for_effort(&effort).await.unwrap().is_empty());
        // Thread-scoped lookup also empty (the row is gone, not orphaned).
        assert!(store.list_for_thread(&thread).await.unwrap().is_empty());
    }
}
