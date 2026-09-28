//! Decisions and claims: the agent's reasoning as reviewable data
//! (`v_decision`, `v_claim`). See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NewDecision {
    pub thread_id: i64,
    pub task_id: Option<i64>,
    pub effort_id: Option<i64>,
    /// The fork: what had to be decided.
    pub question: String,
    /// What was chosen.
    pub choice: String,
    /// The options not taken.
    pub alternatives: Vec<String>,
    /// `low`, `medium` or `high`.
    pub confidence: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NewClaim {
    pub thread_id: i64,
    pub task_id: Option<i64>,
    pub effort_id: Option<i64>,
    pub statement: String,
    /// `tests_pass`, `no_behavior_change`, `handles_case` or `other`.
    pub kind: String,
    /// What backs it (`run:<id>`, a test name, a file); `None` = unbacked.
    pub evidence_ref: Option<String>,
}

#[derive(Clone)]
pub struct SqliteReasoningStore {
    db: Database,
}

impl SqliteReasoningStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record_decision(&self, d: NewDecision) -> Result<i64, DomainError> {
        if !["low", "medium", "high"].contains(&d.confidence.as_str()) {
            return Err(DomainError::Invalid(format!(
                "confidence `{}` must be low, medium or high",
                d.confidence
            )));
        }
        if d.question.trim().is_empty() || d.choice.trim().is_empty() {
            return Err(DomainError::Invalid(
                "a decision needs a question and a choice".into(),
            ));
        }
        let alternatives = serde_json::to_string(&d.alternatives)
            .map_err(|e| DomainError::Storage(format!("alternatives: {e}")))?;
        let now = now_string();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO decision (thread_id, task_id, effort_id, question, choice, alternatives_json, confidence, why, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        d.thread_id, d.task_id, d.effort_id, d.question, d.choice,
                        alternatives, d.confidence, d.why, now
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
    }

    /// Replace `effort_id`'s inferred decisions (provenance `inferred`:
    /// proposed by a model from the effort's activity, not recorded by the
    /// agent) with `decisions`. Recorded decisions are untouched. Their
    /// `effort_id` is overwritten with `effort_id`.
    pub async fn replace_inferred(
        &self,
        effort_id: i64,
        decisions: Vec<NewDecision>,
    ) -> Result<usize, DomainError> {
        let now = now_string();
        let mut rows = Vec::new();
        for d in decisions {
            if d.question.trim().is_empty() || d.choice.trim().is_empty() {
                continue;
            }
            let confidence = if ["low", "medium", "high"].contains(&d.confidence.as_str()) {
                d.confidence
            } else {
                "low".to_string()
            };
            let alternatives = serde_json::to_string(&d.alternatives)
                .map_err(|e| DomainError::Storage(format!("alternatives: {e}")))?;
            rows.push((
                d.thread_id,
                d.task_id,
                d.question,
                d.choice,
                alternatives,
                confidence,
                d.why,
            ));
        }
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "DELETE FROM decision WHERE effort_id = ?1 AND provenance = 'inferred'",
                    [effort_id],
                )
                .map_err(map_sql_err)?;
                for (thread_id, task_id, question, choice, alternatives, confidence, why) in &rows {
                    tx.execute(
                        "INSERT INTO decision (thread_id, task_id, effort_id, question, choice, alternatives_json, confidence, why, provenance, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'inferred', ?9)",
                        rusqlite::params![thread_id, task_id, effort_id, question, choice, alternatives, confidence, why, now],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(rows.len())
            })
            .await
    }

    pub async fn record_claim(&self, c: NewClaim) -> Result<i64, DomainError> {
        if !["tests_pass", "no_behavior_change", "handles_case", "other"].contains(&c.kind.as_str())
        {
            return Err(DomainError::Invalid(format!(
                "claim kind `{}` must be tests_pass, no_behavior_change, handles_case or other",
                c.kind
            )));
        }
        if c.statement.trim().is_empty() {
            return Err(DomainError::Invalid("a claim needs a statement".into()));
        }
        let now = now_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO claim (thread_id, task_id, effort_id, statement, kind, evidence_ref, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![c.thread_id, c.task_id, c.effort_id, c.statement, c.kind, c.evidence_ref, now],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
    }
}

/// RFC 3339 "now", the form every other table stores.
fn now_string() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    async fn seeded() -> (Database, SqliteReasoningStore, SemanticLayer) {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'T', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 1, 'Task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at) VALUES (1, 1, 1, '2026-01-01', '2026-01-01');
                 INSERT INTO task_effort (id, task_id, thread_id, started_at) VALUES (2, 1, 1, '2026-01-02');",
            )
        })
        .await
        .unwrap();
        (
            db.clone(),
            SqliteReasoningStore::new(db.clone()),
            SemanticLayer::new(db),
        )
    }

    /// A test run (a `tests` capture with its `test-detail` payload) on
    /// `effort`, at `at`, with `failed` of `total` cases failing. Returns
    /// the run's id.
    async fn test_run(
        db: &Database,
        effort: Option<i64>,
        failed: i64,
        total: i64,
        at: &str,
    ) -> i64 {
        let at = at.to_string();
        db.call(move |c| {
            let detail = format!(
                "{{\"kind\":\"test-detail\",\"payload\":{{\"passed\":{},\"failed\":{failed},\"total\":{total}}}}}",
                total - failed
            );
            c.execute(
                "INSERT INTO metric_capture (stream_id, thread_id, effort_id, producer, status, provenance, source, captured_at, detail_json)
                 VALUES (1, 1, ?1, 'tests', 'done', 'observed', 'junit', ?2, ?3)",
                rusqlite::params![effort, at, detail],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn decisions_round_trip_through_the_view() {
        let (_db, store, sl) = seeded().await;
        store
            .record_decision(NewDecision {
                thread_id: 1,
                task_id: Some(1),
                effort_id: Some(1),
                question: "Where do extensions live?".into(),
                choice: "oxplow/extensions/".into(),
                alternatives: vec![".oxplow/extensions/".into()],
                confidence: "high".into(),
                why: ".oxplow is gitignored".into(),
            })
            .await
            .unwrap();
        let out = sl
            .query_sql(
                "SELECT task_id, effort_id, choice, alternatives, confidence FROM v_decision",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[
                1,
                1,
                "oxplow/extensions/",
                "[\".oxplow/extensions/\"]",
                "high"
            ]])
        );
    }

    #[tokio::test]
    async fn rejects_bad_confidence_and_kind() {
        let (_db, store, _sl) = seeded().await;
        let err = store
            .record_decision(NewDecision {
                thread_id: 1,
                task_id: None,
                effort_id: None,
                question: "q".into(),
                choice: "c".into(),
                alternatives: vec![],
                confidence: "certain".into(),
                why: String::new(),
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("confidence")),
            "{err:?}"
        );
        let err = store
            .record_claim(NewClaim {
                thread_id: 1,
                task_id: None,
                effort_id: None,
                statement: "s".into(),
                kind: "vibes".into(),
                evidence_ref: None,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("kind")),
            "{err:?}"
        );
    }

    /// `tests_pass` is verified by the effort's LATEST run (its own or one
    /// claimed through attribution) passing with at least one test (tsk366).
    #[tokio::test]
    async fn claims_are_verified_by_evidence_or_the_latest_clean_run() {
        let (db, store, sl) = seeded().await;
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at) VALUES (3, 1, 1, '2026-01-03', '2026-01-03');
                 INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at) VALUES (4, 1, 1, '2026-01-04', '2026-01-04');",
            )
        })
        .await
        .unwrap();
        let claim = |effort: i64, kind: &str, evidence: Option<&str>| NewClaim {
            thread_id: 1,
            task_id: Some(1),
            effort_id: Some(effort),
            statement: format!("{kind} on effort {effort}"),
            kind: kind.into(),
            evidence_ref: evidence.map(str::to_string),
        };
        // Effort 1: passed, then a later failure → not verified.
        test_run(&db, Some(1), 0, 5, "2026-01-01T00:00:00Z").await;
        test_run(&db, Some(1), 1, 5, "2026-01-01T01:00:00Z").await;
        // Effort 2: failed, then fixed → verified.
        test_run(&db, Some(2), 2, 5, "2026-01-02T00:00:00Z").await;
        test_run(&db, Some(2), 0, 5, "2026-01-02T01:00:00Z").await;
        // Effort 3: a clean run it claimed through attribution → verified.
        let run = test_run(&db, None, 0, 5, "2026-01-03T00:00:00Z").await;
        db.call(move |c| {
            c.execute(
                "INSERT INTO effort_attribution (effort_id, kind, ref, state, recorded_at)
                 VALUES (3, 'run', ?1, 'claimed', '2026-01-03')",
                [format!("run:{run}")],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        // Effort 4: a run with no tests → not verified.
        test_run(&db, Some(4), 0, 0, "2026-01-04T00:00:00Z").await;
        for effort in 1..=4 {
            store
                .record_claim(claim(effort, "tests_pass", None))
                .await
                .unwrap();
        }
        store
            .record_claim(claim(2, "handles_case", Some("test:empty_input")))
            .await
            .unwrap(); // evidence
        store
            .record_claim(claim(1, "no_behavior_change", None))
            .await
            .unwrap(); // unbacked
        let out = sl
            .query_sql("SELECT verified FROM v_claim ORDER BY id", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[0], [1], [1], [0], [1], [0]])
        );
    }

    fn decision(question: &str) -> NewDecision {
        NewDecision {
            thread_id: 1,
            task_id: Some(1),
            effort_id: Some(1),
            question: question.into(),
            choice: "c".into(),
            alternatives: vec![],
            confidence: "medium".into(),
            why: "w".into(),
        }
    }

    #[tokio::test]
    async fn inferred_decisions_are_marked_and_replaced_per_effort() {
        let (_db, store, sl) = seeded().await;
        store
            .record_decision(decision("recorded one"))
            .await
            .unwrap();
        store
            .replace_inferred(1, vec![decision("guess a"), decision("guess b")])
            .await
            .unwrap();
        store
            .replace_inferred(2, vec![decision("other effort")])
            .await
            .unwrap();
        // A second pass replaces the first; the recorded one and effort 2's stay.
        let n = store
            .replace_inferred(1, vec![decision("guess c")])
            .await
            .unwrap();
        assert_eq!(n, 1);
        let out = sl
            .query_sql(
                "SELECT effort_id, question, provenance FROM v_decision ORDER BY effort_id, id",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([
                [1, "recorded one", "recorded"],
                [1, "guess c", "inferred"],
                [2, "other effort", "inferred"]
            ])
        );
        // Bad confidence is normalized rather than failing the whole batch.
        let mut odd = decision("odd");
        odd.confidence = "very".into();
        store.replace_inferred(1, vec![odd]).await.unwrap();
        let out = sl
            .query_sql(
                "SELECT confidence FROM v_decision WHERE question = 'odd'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(&out.rows).unwrap(), json!([["low"]]));
    }
}
