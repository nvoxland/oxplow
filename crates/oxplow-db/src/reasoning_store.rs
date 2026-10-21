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
    /// The work item it was made on (`oxplow:oxplow/work_item?id=<id>&provider=<provider>`).
    pub work_item: Option<String>,
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
    /// The work item it was made on (`oxplow:oxplow/work_item?id=<id>&provider=<provider>`).
    pub work_item: Option<String>,
    pub effort_id: Option<i64>,
    pub statement: String,
    /// `tests_pass`, `no_behavior_change`, `handles_case` or `other`.
    pub kind: String,
    /// What backs it (`oxplow:oxplow/run?id=<id>`, a test name, a file); `None` = unbacked.
    pub evidence_ref: Option<String>,
    /// The agent session making it: it lands on that session's open turn.
    pub agent_session_id: Option<i64>,
}

#[derive(Clone)]
pub struct SqliteReasoningStore {
    db: Database,
}

impl SqliteReasoningStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace `effort_id`'s inferred decisions (`replace_inferred_tx`) in
    /// a transaction of its own.
    pub async fn replace_inferred(
        &self,
        effort_id: i64,
        decisions: Vec<NewDecision>,
    ) -> Result<usize, DomainError> {
        self.db
            .transaction(move |tx| replace_inferred_tx(tx, effort_id, &decisions))
            .await
    }
}

/// Replace `effort_id`'s inferred decisions (provenance `inferred`:
/// proposed by a model from the effort's activity, not recorded by the
/// agent) with `decisions`, in the caller's transaction. Recorded,
/// confirmed and dismissed decisions are untouched. Entries without a
/// question or choice are skipped, an unknown confidence is `low`, and
/// each row's `effort_id` is `effort_id`. Returns how many were stored.
pub fn replace_inferred_tx(
    conn: &rusqlite::Connection,
    effort_id: i64,
    decisions: &[NewDecision],
) -> Result<usize, DomainError> {
    conn.execute(
        "DELETE FROM decision WHERE effort_id = ?1 AND provenance = 'inferred'",
        [effort_id],
    )
    .map_err(map_sql_err)?;
    let now = now_string();
    let mut stored = 0;
    for d in decisions {
        if d.question.trim().is_empty() || d.choice.trim().is_empty() {
            continue;
        }
        let confidence = if ["low", "medium", "high"].contains(&d.confidence.as_str()) {
            d.confidence.as_str()
        } else {
            "low"
        };
        let alternatives = serde_json::to_string(&d.alternatives)
            .map_err(|e| DomainError::Storage(format!("alternatives: {e}")))?;
        conn.execute(
            "INSERT INTO decision (thread_id, work_item, effort_id, question, choice, alternatives_json, confidence, why, provenance, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'inferred', ?9)",
            rusqlite::params![d.thread_id, d.work_item, effort_id, d.question, d.choice, alternatives, confidence, d.why, now],
        )
        .map_err(map_sql_err)?;
        stored += 1;
    }
    Ok(stored)
}

/// Record a decision the agent made, in the caller's transaction (the
/// `oxplow.effort.record_decision` command). Made in the thread's open turn, when
/// one is open.
pub fn record_decision_tx(
    conn: &rusqlite::Connection,
    d: &NewDecision,
) -> Result<i64, DomainError> {
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
    conn.execute(
        "INSERT INTO decision (thread_id, work_item, effort_id, question, choice, alternatives_json, confidence, why, created_at, turn_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, (SELECT id FROM agent_turn
            WHERE thread_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC, id DESC LIMIT 1))",
        rusqlite::params![
            d.thread_id, d.work_item, d.effort_id, d.question, d.choice,
            alternatives, d.confidence, d.why, now_string()
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// Where a claim came from: the agent stated it (`oxplow.effort.report`'s
/// `claims`) or a model read it in the effort's answers
/// (`oxplow.effort.replace_inferred_claims`). The review treats both alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimProvenance {
    Recorded,
    Inferred,
}

impl ClaimProvenance {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimProvenance::Recorded => "recorded",
            ClaimProvenance::Inferred => "inferred",
        }
    }
}

/// Insert a claim about the agent's work, in the caller's transaction.
pub fn insert_claim_tx(
    conn: &rusqlite::Connection,
    provenance: ClaimProvenance,
    c: &NewClaim,
) -> Result<i64, DomainError> {
    if !["tests_pass", "no_behavior_change", "handles_case", "other"].contains(&c.kind.as_str()) {
        return Err(DomainError::Invalid(format!(
            "claim kind `{}` must be tests_pass, no_behavior_change, handles_case or other",
            c.kind
        )));
    }
    if c.statement.trim().is_empty() {
        return Err(DomainError::Invalid("a claim needs a statement".into()));
    }
    conn.execute(
        "INSERT INTO claim (thread_id, work_item, effort_id, statement, kind, evidence_ref, created_at, agent_session_id, provenance)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![c.thread_id, c.work_item, c.effort_id, c.statement, c.kind, c.evidence_ref, now_string(), c.agent_session_id, provenance.as_str()],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// Replace effort `effort`'s claims from `provenance` with `claims`, in the
/// caller's transaction: the other source's claims stay, and so do the ones
/// a reviewer verified or dismissed (`review`). Returns how many it stored.
pub fn replace_claims_tx(
    conn: &rusqlite::Connection,
    effort: i64,
    provenance: ClaimProvenance,
    claims: &[NewClaim],
) -> Result<usize, DomainError> {
    conn.execute(
        "DELETE FROM claim WHERE effort_id = ?1 AND provenance = ?2 AND review IS NULL",
        rusqlite::params![effort, provenance.as_str()],
    )
    .map_err(map_sql_err)?;
    for claim in claims {
        insert_claim_tx(conn, provenance, claim)?;
    }
    Ok(claims.len())
}

/// The cores, run on the store's own database — for this module's tests.
#[cfg(test)]
impl SqliteReasoningStore {
    async fn record_decision(&self, d: NewDecision) -> Result<i64, DomainError> {
        self.db
            .transaction(move |tx| record_decision_tx(tx, &d))
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
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                   VALUES (1, 1, 'T', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 1, 'Task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at, ended_at) VALUES (1, 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow', 1, '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at) VALUES (2, 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow', 1, '2026-01-02');",
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

    #[tokio::test]
    async fn decisions_round_trip_through_the_view() {
        let (_db, store, sl) = seeded().await;
        store
            .record_decision(NewDecision {
                thread_id: 1,
                work_item: Some("oxplow:oxplow/work_item?id=tsk1&provider=oxplow".into()),
                effort_id: Some(1),
                question: "Where do extensions live?".into(),
                choice: ".oxplow/extensions/".into(),
                alternatives: vec![".oxplow/extensions/".into()],
                confidence: "high".into(),
                why: ".oxplow is gitignored".into(),
            })
            .await
            .unwrap();
        let out = sl
            .query_sql(
                "SELECT work_item, effort_id, choice, alternatives, confidence FROM v_decision",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([[
                "oxplow:oxplow/work_item?id=tsk1&provider=oxplow",
                1,
                ".oxplow/extensions/",
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
                work_item: None,
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
        let mut vibes = claim("s");
        vibes.kind = "vibes".into();
        let err = store
            .db
            .transaction(move |tx| insert_claim_tx(tx, ClaimProvenance::Recorded, &vibes))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("kind")),
            "{err:?}"
        );
    }

    fn decision(question: &str) -> NewDecision {
        NewDecision {
            thread_id: 1,
            work_item: Some("oxplow:oxplow/work_item?id=tsk1&provider=oxplow".into()),
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

    fn claim(statement: &str) -> NewClaim {
        NewClaim {
            thread_id: 1,
            work_item: Some("oxplow:oxplow/work_item?id=tsk1&provider=oxplow".into()),
            effort_id: Some(1),
            statement: statement.into(),
            kind: "tests_pass".into(),
            evidence_ref: None,
            agent_session_id: None,
        }
    }

    /// An effort's claims come from two sources, one record: what the
    /// agent stated (`recorded`) and what a model read in its answers
    /// (`inferred`). Replacing one source's claims leaves the other's, and
    /// a reviewed claim of either stays.
    #[tokio::test]
    async fn each_source_replaces_only_its_own_unreviewed_claims() {
        use ClaimProvenance::{Inferred, Recorded};
        let (db, _, layer) = seeded().await;
        db.transaction(|tx| {
            replace_claims_tx(
                tx,
                1,
                Recorded,
                &[claim("stated"), claim("stated, verified")],
            )?;
            replace_claims_tx(tx, 1, Inferred, &[claim("read"), claim("read, dismissed")])?;
            tx.execute(
                "UPDATE claim SET review = 'verified' WHERE statement = 'stated, verified'",
                [],
            )
            .map_err(map_sql_err)?;
            tx.execute(
                "UPDATE claim SET review = 'dismissed' WHERE statement = 'read, dismissed'",
                [],
            )
            .map_err(map_sql_err)?;
            replace_claims_tx(tx, 1, Inferred, &[claim("read again")])?;
            replace_claims_tx(tx, 1, Recorded, &[claim("stated again")])
        })
        .await
        .unwrap();
        let out = layer
            .query_sql(
                "SELECT statement, provenance, review FROM v_claim ORDER BY statement",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([
                ["read again", "inferred", null],
                ["read, dismissed", "inferred", "dismissed"],
                ["stated again", "recorded", null],
                ["stated, verified", "recorded", "verified"],
            ])
        );
    }
}
