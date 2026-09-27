//! Decisions and claims: the agent's reasoning as reviewable data
//! (`v_decision`, `v_claim`). See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

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

    /// A `tests` capture on `effort` with test cases of the given statuses.
    async fn test_report(db: &Database, effort: i64, statuses: &'static [&'static str]) {
        db.call(move |c| {
            c.execute(
                "INSERT INTO metric_capture (stream_id, thread_id, effort_id, producer, status, provenance, source, captured_at)
                 VALUES (1, 1, ?1, 'tests', 'done', 'observed', 'junit', '2026-01-01')",
                [effort],
            )?;
            let cap = c.last_insert_rowid();
            let measure: i64 = c.query_row("SELECT id FROM measure WHERE key = 'oxplow.test_case'", [], |r| r.get(0))?;
            for s in statuses {
                c.execute(
                    "INSERT INTO fact (capture_id, measure_id, value, dims_json) VALUES (?1, ?2, 1, ?3)",
                    rusqlite::params![cap, measure, format!("{{\"oxplow.status\":\"{s}\"}}")],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
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

    #[tokio::test]
    async fn claims_are_verified_by_evidence_or_a_clean_test_report() {
        let (db, store, sl) = seeded().await;
        let claim = |effort: i64, kind: &str, evidence: Option<&str>| NewClaim {
            thread_id: 1,
            task_id: Some(1),
            effort_id: Some(effort),
            statement: format!("{kind} on effort {effort}"),
            kind: kind.into(),
            evidence_ref: evidence.map(str::to_string),
        };
        test_report(&db, 1, &["passed", "passed"]).await;
        test_report(&db, 2, &["passed", "failed"]).await;
        store
            .record_claim(claim(1, "tests_pass", None))
            .await
            .unwrap(); // clean report → verified
        store
            .record_claim(claim(2, "tests_pass", None))
            .await
            .unwrap(); // failing report → not
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
            json!([[1], [0], [1], [0]])
        );
    }
}
