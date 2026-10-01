//! `command_proposal` (migration V127, P6b): a command an agent ran that
//! needs a person's confirmation, kept with its preview until a person
//! approves or declines it. Written inside the bus's transactions; read
//! here and as `v_command_proposal`. See `.context/commands.md`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use oxplow_domain::events::schema::ActorKind;
use oxplow_domain::{DomainError, StreamId, ThreadId, Timestamp};

use crate::command_audit_store::{actor_kind_str, parse_actor_kind};
use crate::database::{map_sql_err, string_to_ts, ts_to_string, Database};
use crate::event_content_store::canonical;

/// Where a proposal stands. Only a `Pending` one can be decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    Pending,
    Approved,
    Declined,
    /// A newer proposal with the same key replaced it.
    Superseded,
}

impl ProposalDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Declined => "declined",
            Self::Superseded => "superseded",
        }
    }

    fn parse(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "declined" => Self::Declined,
            "superseded" => Self::Superseded,
            other => return Err(DomainError::Invalid(format!("decision `{other}`"))),
        })
    }
}

/// One proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Proposal {
    pub id: i64,
    pub created_at: Timestamp,
    pub command: String,
    pub input: Value,
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub thread_id: Option<ThreadId>,
    pub stream_id: Option<StreamId>,
    /// What it supersedes on: [`proposal_key`].
    pub key: String,
    /// The confirmation preview the run raised.
    pub preview: Value,
    /// What the run would have done when it was proposed (a config key's
    /// before/after, a composite's children); `None` when it can't be
    /// dry-run.
    pub dry_run: Option<Value>,
    pub decision: ProposalDecision,
    pub decided_at: Option<Timestamp>,
    /// The approving run's `command_audit` row.
    pub audit_id: Option<i64>,
    /// The proposal that replaced this one.
    pub superseded_by: Option<i64>,
}

/// What a proposal is stored from.
#[derive(Debug, Clone)]
pub struct NewProposal {
    pub command: String,
    pub input: Value,
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub thread_id: Option<ThreadId>,
    pub stream_id: Option<StreamId>,
    pub preview: Value,
    pub dry_run: Option<Value>,
}

/// What a newer proposal replaces an older pending one on: the config key
/// for `config.set` / `config.unset` (`config:<key>`), else the command and
/// its input with keys sorted.
pub fn proposal_key(command: &str, input: &Value) -> String {
    match (command, input.get("key").and_then(Value::as_str)) {
        ("config.set" | "config.unset", Some(key)) => format!("config:{key}"),
        _ => format!("{command} {}", canonical(input)),
    }
}

/// Store a pending proposal; its id. Pending proposals with the same key
/// are marked superseded by it.
pub fn insert_tx(conn: &Connection, row: &NewProposal) -> Result<Inserted, DomainError> {
    let key = proposal_key(&row.command, &row.input);
    let now = ts_to_string(Timestamp::now());
    conn.execute(
        "INSERT INTO command_proposal
           (created_at, command, input_json, actor_kind, actor_id, thread_id, stream_id, key,
            preview_json, dry_run_json, decision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending')",
        params![
            now,
            row.command,
            row.input.to_string(),
            actor_kind_str(row.actor_kind),
            row.actor_id,
            row.thread_id.map(|t| t.value()),
            row.stream_id.map(|s| s.value()),
            key,
            row.preview.to_string(),
            row.dry_run.as_ref().map(Value::to_string),
        ],
    )
    .map_err(map_sql_err)?;
    let id = conn.last_insert_rowid();
    let mut stmt = conn
        .prepare(
            "UPDATE command_proposal
                SET decision = 'superseded', decided_at = ?3, superseded_by = ?1
              WHERE key = ?2 AND decision = 'pending' AND id <> ?1
              RETURNING id",
        )
        .map_err(map_sql_err)?;
    let mut superseded = stmt
        .query_map(params![id, key, now], |r| r.get::<_, i64>(0))
        .map_err(map_sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sql_err)?;
    superseded.sort_unstable();
    Ok(Inserted { id, superseded })
}

/// A proposal just stored, and the pending ones with its key that it
/// replaced (marked `superseded`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inserted {
    pub id: i64,
    pub superseded: Vec<i64>,
}

/// Mark a pending proposal `decision`; the row as it now is. Deciding a
/// missing proposal is `NotFound`; one already decided is `Invalid`.
fn decide_tx(
    conn: &Connection,
    id: i64,
    decision: ProposalDecision,
    audit_id: Option<i64>,
) -> Result<Proposal, DomainError> {
    let current = get_tx(conn, id)?.ok_or(DomainError::NotFound)?;
    if current.decision != ProposalDecision::Pending {
        return Err(DomainError::Invalid(format!(
            "proposal:{id} is already {}",
            current.decision.as_str()
        )));
    }
    conn.execute(
        "UPDATE command_proposal SET decision = ?2, decided_at = ?3, audit_id = ?4 WHERE id = ?1",
        params![
            id,
            decision.as_str(),
            ts_to_string(Timestamp::now()),
            audit_id
        ],
    )
    .map_err(map_sql_err)?;
    get_tx(conn, id)?.ok_or(DomainError::NotFound)
}

/// A person approved it and the run is audited as `audit_id`.
pub fn approve_tx(conn: &Connection, id: i64, audit_id: i64) -> Result<Proposal, DomainError> {
    decide_tx(conn, id, ProposalDecision::Approved, Some(audit_id))
}

/// Before an `External` approval runs: mark it approved with no audit row
/// yet, so a second approval can't run it too. [`finish_claim_tx`] names
/// the run; [`release_claim_tx`] undoes the claim when the run fails.
pub fn claim_tx(conn: &Connection, id: i64) -> Result<(), DomainError> {
    decide_tx(conn, id, ProposalDecision::Approved, None).map(|_| ())
}

/// The claimed approval ran, audited as `audit_id`.
pub fn finish_claim_tx(conn: &Connection, id: i64, audit_id: i64) -> Result<(), DomainError> {
    let n = conn
        .execute(
            "UPDATE command_proposal SET audit_id = ?2
              WHERE id = ?1 AND decision = 'approved' AND audit_id IS NULL",
            params![id, audit_id],
        )
        .map_err(map_sql_err)?;
    if n == 0 {
        return Err(DomainError::Invariant(format!(
            "proposal:{id} is not claimed for approval"
        )));
    }
    Ok(())
}

/// The claimed approval's run failed: it is pending again.
pub fn release_claim_tx(conn: &Connection, id: i64) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE command_proposal SET decision = 'pending', decided_at = NULL
          WHERE id = ?1 AND decision = 'approved' AND audit_id IS NULL",
        params![id],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// A person declined it; nothing ran.
pub fn decline_tx(conn: &Connection, id: i64) -> Result<Proposal, DomainError> {
    decide_tx(conn, id, ProposalDecision::Declined, None)
}

fn row_to_proposal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Proposal> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let json = |col: &str, s: String| {
        serde_json::from_str::<Value>(&s)
            .map_err(|e| conv(DomainError::Storage(format!("{col}: {e}"))))
    };
    let created_at: String = row.get("created_at")?;
    let decided_at: Option<String> = row.get("decided_at")?;
    let actor_kind: String = row.get("actor_kind")?;
    let decision: String = row.get("decision")?;
    let dry_run: Option<String> = row.get("dry_run_json")?;
    Ok(Proposal {
        id: row.get("id")?,
        created_at: string_to_ts(&created_at).map_err(conv)?,
        command: row.get("command")?,
        input: json("input_json", row.get("input_json")?)?,
        actor_kind: parse_actor_kind(&actor_kind).map_err(conv)?,
        actor_id: row.get("actor_id")?,
        thread_id: row.get::<_, Option<i64>>("thread_id")?.map(ThreadId::new),
        stream_id: row.get::<_, Option<i64>>("stream_id")?.map(StreamId::new),
        key: row.get("key")?,
        preview: json("preview_json", row.get("preview_json")?)?,
        dry_run: dry_run.map(|s| json("dry_run_json", s)).transpose()?,
        decision: ProposalDecision::parse(&decision).map_err(conv)?,
        decided_at: decided_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(conv)?,
        audit_id: row.get("audit_id")?,
        superseded_by: row.get("superseded_by")?,
    })
}

pub fn get_tx(conn: &Connection, id: i64) -> Result<Option<Proposal>, DomainError> {
    conn.query_row(
        "SELECT * FROM command_proposal WHERE id = ?1",
        params![id],
        row_to_proposal,
    )
    .optional()
    .map_err(map_sql_err)
}

/// The pending proposals, newest first.
pub fn list_pending_tx(conn: &Connection) -> Result<Vec<Proposal>, DomainError> {
    let mut stmt = conn
        .prepare("SELECT * FROM command_proposal WHERE decision = 'pending' ORDER BY id DESC")
        .map_err(map_sql_err)?;
    let rows = stmt.query_map([], row_to_proposal).map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Async wrappers over the `_tx` cores. The bus writes through the cores
/// inside its own transaction; these are for reads and tests.
#[derive(Clone)]
pub struct SqliteProposalStore {
    db: Database,
}

impl SqliteProposalStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: i64) -> Result<Option<Proposal>, DomainError> {
        self.db.call_mut(move |c| get_tx(c, id)).await
    }

    pub async fn list_pending(&self) -> Result<Vec<Proposal>, DomainError> {
        self.db.call_mut(|c| list_pending_tx(c)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A database with stream 1 and its thread 3, which the proposals name.
    async fn seeded() -> Database {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (3, 1, 'T', 'active', '2026-01-01', '2026-01-01');",
            )
        })
        .await
        .unwrap();
        db
    }

    fn config_set(key: &str, value: Value) -> NewProposal {
        NewProposal {
            command: "config.set".into(),
            input: json!({ "key": key, "value": value }),
            actor_kind: ActorKind::Agent,
            actor_id: Some("thr3".into()),
            thread_id: Some(ThreadId::new(3)),
            stream_id: Some(StreamId::new(1)),
            preview: json!({ "summary": format!("Set {key}") }),
            dry_run: Some(json!({ "before": null, "after": value })),
        }
    }

    #[test]
    fn the_key_is_the_config_key_or_the_command_and_its_sorted_input() {
        assert_eq!(
            proposal_key("config.set", &json!({ "key": "zones", "value": [] })),
            "config:zones"
        );
        assert_eq!(
            proposal_key("config.unset", &json!({ "key": "zones" })),
            "config:zones"
        );
        let a = proposal_key("work_item.delete", &json!({ "ref": "r", "hard": true }));
        let b = proposal_key("work_item.delete", &json!({ "hard": true, "ref": "r" }));
        assert_eq!(a, b);
        assert_eq!(a, r#"work_item.delete {"hard":true,"ref":"r"}"#);
    }

    #[tokio::test]
    async fn a_proposal_round_trips() {
        let db = seeded().await;
        let store = SqliteProposalStore::new(db.clone());
        let id = db
            .transaction(|tx| insert_tx(tx, &config_set("agentPromptAppend", json!("hi"))))
            .await
            .unwrap()
            .id;
        let p = store.get(id).await.unwrap().unwrap();
        assert_eq!(p.command, "config.set");
        assert_eq!(p.input["value"], "hi");
        assert_eq!(p.actor_kind, ActorKind::Agent);
        assert_eq!(p.actor_id.as_deref(), Some("thr3"));
        assert_eq!(p.thread_id, Some(ThreadId::new(3)));
        assert_eq!(p.stream_id, Some(StreamId::new(1)));
        assert_eq!(p.key, "config:agentPromptAppend");
        assert_eq!(p.preview["summary"], "Set agentPromptAppend");
        assert_eq!(p.dry_run, Some(json!({ "before": null, "after": "hi" })));
        assert_eq!(p.decision, ProposalDecision::Pending);
        assert_eq!(
            (p.decided_at, p.audit_id, p.superseded_by),
            (None, None, None)
        );
        assert_eq!(store.list_pending().await.unwrap(), vec![p]);
    }

    #[tokio::test]
    async fn a_newer_proposal_supersedes_the_pending_one_with_its_key() {
        let db = seeded().await;
        let store = SqliteProposalStore::new(db.clone());
        let (old, other, new) = db
            .transaction(|tx| {
                let old = insert_tx(tx, &config_set("zones", json!([])))?;
                let other = insert_tx(tx, &config_set("agentPromptAppend", json!("x")))?;
                let new = insert_tx(tx, &config_set("zones", json!(["a"])))?;
                assert!(old.superseded.is_empty() && other.superseded.is_empty());
                assert_eq!(new.superseded, vec![old.id], "it names what it replaced");
                Ok((old.id, other.id, new.id))
            })
            .await
            .unwrap();
        let replaced = store.get(old).await.unwrap().unwrap();
        assert_eq!(replaced.decision, ProposalDecision::Superseded);
        assert_eq!(replaced.superseded_by, Some(new));
        assert!(replaced.decided_at.is_some());
        let pending: Vec<i64> = store
            .list_pending()
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(pending, vec![new, other], "newest first");
        // A superseded proposal can't be decided.
        let err = db
            .transaction(move |tx| decline_tx(tx, old))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("superseded")),
            "{err:?}"
        );
    }

    /// An `External` approval claims the proposal before it runs, then
    /// finishes the claim with its audit row — or releases it when the
    /// run failed, so it is pending again. A claimed proposal can't be
    /// claimed or declined twice.
    #[tokio::test]
    async fn an_approval_claims_then_finishes_or_releases() {
        let db = seeded().await;
        let store = SqliteProposalStore::new(db.clone());
        let id = db
            .transaction(|tx| insert_tx(tx, &config_set("zones", json!([]))))
            .await
            .unwrap()
            .id;
        db.transaction(move |tx| claim_tx(tx, id)).await.unwrap();
        let claimed = store.get(id).await.unwrap().unwrap();
        assert_eq!(claimed.decision, ProposalDecision::Approved);
        assert_eq!(claimed.audit_id, None);
        assert!(db.transaction(move |tx| claim_tx(tx, id)).await.is_err());
        assert!(db.transaction(move |tx| decline_tx(tx, id)).await.is_err());

        db.transaction(move |tx| release_claim_tx(tx, id))
            .await
            .unwrap();
        let released = store.get(id).await.unwrap().unwrap();
        assert_eq!(released.decision, ProposalDecision::Pending);
        assert_eq!(released.decided_at, None);

        db.transaction(move |tx| claim_tx(tx, id)).await.unwrap();
        let audit = db
            .transaction(|tx| {
                crate::command_audit_store::insert_tx(
                    tx,
                    &crate::NewCommandAudit {
                        command: "config.set".into(),
                        actor_kind: ActorKind::Human,
                        actor_id: None,
                        thread_id: None,
                        input: json!({}),
                        outcome: oxplow_domain::events::schema::CommandOutcome::Ok,
                        error: None,
                        result: None,
                        inverse: None,
                    },
                )
            })
            .await
            .unwrap();
        db.transaction(move |tx| finish_claim_tx(tx, id, audit))
            .await
            .unwrap();
        assert_eq!(store.get(id).await.unwrap().unwrap().audit_id, Some(audit));
        // Finished: neither finished again nor released.
        assert!(db
            .transaction(move |tx| finish_claim_tx(tx, id, audit))
            .await
            .is_err());
        db.transaction(move |tx| release_claim_tx(tx, id))
            .await
            .unwrap();
        assert_eq!(
            store.get(id).await.unwrap().unwrap().decision,
            ProposalDecision::Approved
        );
    }

    #[tokio::test]
    async fn a_proposal_is_decided_once() {
        let db = seeded().await;
        let store = SqliteProposalStore::new(db.clone());
        let (approved, declined) = db
            .transaction(|tx| {
                let a = insert_tx(tx, &config_set("zones", json!([])))?;
                let d = insert_tx(tx, &config_set("agentPromptAppend", json!("x")))?;
                Ok((a.id, d.id))
            })
            .await
            .unwrap();
        // The audit row an approval names exists (the approving run's).
        let audit = db
            .transaction(|tx| {
                crate::command_audit_store::insert_tx(
                    tx,
                    &crate::NewCommandAudit {
                        command: "config.set".into(),
                        actor_kind: ActorKind::Human,
                        actor_id: None,
                        thread_id: None,
                        input: json!({}),
                        outcome: oxplow_domain::events::schema::CommandOutcome::Ok,
                        error: None,
                        result: None,
                        inverse: None,
                    },
                )
            })
            .await
            .unwrap();
        let a = db
            .transaction(move |tx| approve_tx(tx, approved, audit))
            .await
            .unwrap();
        assert_eq!(a.decision, ProposalDecision::Approved);
        assert_eq!(a.audit_id, Some(audit));
        assert!(a.decided_at.is_some());
        let d = db
            .transaction(move |tx| decline_tx(tx, declined))
            .await
            .unwrap();
        assert_eq!((d.decision, d.audit_id), (ProposalDecision::Declined, None));
        assert!(store.list_pending().await.unwrap().is_empty());
        for (id, done) in [(approved, "approved"), (declined, "declined")] {
            let err = db
                .transaction(move |tx| decline_tx(tx, id))
                .await
                .unwrap_err();
            assert!(
                matches!(err, DomainError::Invalid(ref m) if m.contains(done)),
                "{err:?}"
            );
        }
        let missing = db.transaction(|tx| decline_tx(tx, 999)).await.unwrap_err();
        assert!(matches!(missing, DomainError::NotFound), "{missing:?}");
    }
}
