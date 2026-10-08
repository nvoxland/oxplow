//! oxplow's tasks as a work-items provider: their verbs, called like any
//! other list's ([`WorkItemVerbs`]) — each in its own transaction over
//! the task tables, answering with the `work_item.recorded` of every task
//! it changed, which is how what it did reaches the interface.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use oxplow_db::Database;
use oxplow_domain::events::schema::{WorkItemRecorded, WorkItemRecordedV2};
use oxplow_domain::work_items::{VerbOutcome, WorkItemRecord, WorkItemVerbs};
use oxplow_domain::{Actor, CommandError, DomainError, Envelope};

use crate::verbs::{self, Answer};

/// `item`'s `work_item.recorded`, as oxplow's tasks answer with it.
pub fn recorded(item: WorkItemRecord) -> Envelope {
    let subject = item.item_ref.clone();
    Envelope::typed::<WorkItemRecorded>("provider:oxplow", &WorkItemRecordedV2 { item })
        .with_subject([subject])
}

/// oxplow's task list's verbs over `db`.
pub struct OxplowTasks {
    db: Database,
}

impl OxplowTasks {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

/// Run `verb` in the caller's transaction: its answer and the records of
/// the tasks it changed.
fn run_tx(
    conn: &rusqlite::Connection,
    actor: &Actor,
    verb: &str,
    input: Value,
) -> Result<(Answer, Vec<WorkItemRecord>), CommandError> {
    let answer = match verb {
        "create" => verbs::create_tx(conn, actor, parse(input)?),
        "update" => verbs::update_tx(conn, parse(input)?),
        "transition" => verbs::transition_tx(conn, parse(input)?),
        "link" => verbs::link_tx(conn, actor, parse(input)?),
        "comment" => verbs::comment_tx(conn, actor, parse(input)?),
        "delete" => verbs::delete_tx(conn, parse(input)?),
        "reorder" => verbs::reorder_tx(conn, parse(input)?),
        "move" => verbs::move_tx(conn, parse(input)?),
        other => Err(CommandError::Invalid {
            field: None,
            message: format!("oxplow's tasks have no verb `{other}`"),
        }),
    }?;
    let records = answer
        .changed
        .iter()
        .map(|&id| crate::record::record_tx(conn, id))
        .collect::<Result<Vec<_>, DomainError>>()
        .map_err(CommandError::from)?;
    Ok((answer, records))
}

#[async_trait]
impl WorkItemVerbs for OxplowTasks {
    /// Its writes are one transaction: a refused verb leaves nothing. It
    /// keeps no idempotency keys (it declares no `idempotent_writes`).
    async fn invoke(
        &self,
        actor: &Actor,
        verb: &str,
        input: Value,
        _idempotency_key: Option<String>,
    ) -> Result<VerbOutcome, CommandError> {
        let (actor, verb) = (actor.clone(), verb.to_string());
        // A refusal rolls the transaction back: it's kept here and the
        // transaction is failed.
        let refused: Arc<Mutex<Option<CommandError>>> = Arc::default();
        let kept = refused.clone();
        let ran = self
            .db
            .transaction(move |tx| {
                run_tx(tx, &actor, &verb, input.clone()).map_err(|e| {
                    *kept.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
                    DomainError::Invalid("the verb was refused".into())
                })
            })
            .await;
        let (answer, records) = match ran {
            Ok(done) => done,
            Err(e) => {
                let refusal = refused.lock().unwrap_or_else(|p| p.into_inner()).take();
                return Err(refusal.unwrap_or_else(|| CommandError::from(e)));
            }
        };
        let events = records.into_iter().map(recorded).collect();
        Ok(VerbOutcome {
            result: answer.result,
            events,
            inverse: answer.inverse,
        })
    }

    /// Nothing to restart: it keeps no process.
    async fn restart(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn tasks() -> (OxplowTasks, Database) {
        let db = Database::in_memory();
        (OxplowTasks::new(db.clone()), db)
    }

    /// A verb answers with the record of what it changed — the item whole,
    /// on its list — and its inverse by verb.
    #[tokio::test]
    async fn a_verb_answers_with_the_records_of_what_it_changed() {
        let (tasks, _db) = tasks().await;
        let created = tasks
            .invoke(&Actor::Human, "create", json!({ "title": "t" }), None)
            .await
            .unwrap();
        let item = created.result["ref"].as_str().unwrap().to_string();
        assert_eq!(created.events.len(), 1);
        let WorkItemRecordedV2 { item: record } =
            serde_json::from_value(created.events[0].payload.clone()).unwrap();
        assert_eq!(record.item_ref, item);
        assert_eq!(record.list, Some(oxplow_domain::work_items::List::Backlog));
        assert_eq!(created.events[0].subject, vec![item.clone()]);

        let moved = tasks
            .invoke(
                &Actor::Human,
                "transition",
                json!({ "ref": item, "to": "done" }),
                None,
            )
            .await
            .unwrap();
        let WorkItemRecordedV2 { item: record } =
            serde_json::from_value(moved.events[0].payload.clone()).unwrap();
        assert_eq!(record.state.as_str(), "done");
        assert_eq!(moved.inverse.unwrap().name, "transition");
    }

    /// A refused verb writes nothing: its transaction rolls back, and the
    /// refusal is the verb's own.
    #[tokio::test]
    async fn a_refused_verb_leaves_nothing() {
        let (tasks, db) = tasks().await;
        let err = tasks
            .invoke(
                &Actor::Human,
                "create",
                json!({ "title": "t", "native_state": "nope" }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field, .. } if field.as_deref() == Some("/native_state")),
            "{err:?}"
        );
        let rows: i64 = db
            .read(|c| {
                c.query_row("SELECT count(*) FROM task", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}
