//! oxplow's tasks as a work-items provider: their verbs, called like any
//! other list's ([`WorkItemVerbs`]) — each in its own transaction over
//! the task tables, answering with the `work_item.recorded` of every task
//! it changed, which is how what it did reaches the interface.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use oxplow_db::Database;
use oxplow_domain::events::schema::{WorkItemRecorded, WorkItemRecordedV2};
use oxplow_domain::work_items::{VerbCall, VerbOutcome, WorkItemRecord, WorkItemVerbs};
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
        call: VerbCall<'_>,
        verb: &str,
        input: Value,
    ) -> Result<VerbOutcome, CommandError> {
        let (actor, verb) = (call.actor.clone(), verb.to_string());
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
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "create",
                json!({ "title": "t" }),
            )
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
                VerbCall::bare(&Actor::Human, None),
                "transition",
                json!({ "ref": item, "to": "done" }),
            )
            .await
            .unwrap();
        let WorkItemRecordedV2 { item: record } =
            serde_json::from_value(moved.events[0].payload.clone()).unwrap();
        assert_eq!(record.state.as_str(), "done");
        assert_eq!(moved.inverse.unwrap().name, "transition");
    }

    fn recorded_state(out: &VerbOutcome) -> (String, String) {
        let WorkItemRecordedV2 { item } =
            serde_json::from_value(out.events[0].payload.clone()).unwrap();
        (item.state.as_str().to_string(), item.native_state)
    }

    /// `done` + `archived` means a completed task that was archived: a
    /// create or an update asked for it records `done`, as a transition
    /// does, not `canceled`.
    #[tokio::test]
    async fn filing_or_updating_into_done_archived_reads_as_done() {
        let (tasks, _db) = tasks().await;
        let filed = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "create",
                json!({ "title": "t", "state": "done", "native_state": "archived" }),
            )
            .await
            .unwrap();
        assert_eq!(recorded_state(&filed), ("done".into(), "archived".into()));

        let plain = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "create",
                json!({ "title": "u" }),
            )
            .await
            .unwrap();
        let item = plain.result["ref"].as_str().unwrap().to_string();
        let updated = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "update",
                json!({ "ref": item, "state": "done", "native_state": "archived" }),
            )
            .await
            .unwrap();
        assert_eq!(recorded_state(&updated), ("done".into(), "archived".into()));
    }

    /// An update's inverse names the task's prior status whole, so undoing
    /// a state change on an archived task archives it again.
    #[tokio::test]
    async fn undoing_a_state_update_on_an_archived_task_restores_archived() {
        let (tasks, _db) = tasks().await;
        let filed = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "create",
                json!({ "title": "t", "state": "done", "native_state": "archived" }),
            )
            .await
            .unwrap();
        let item = filed.result["ref"].as_str().unwrap().to_string();
        let reopened = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "update",
                json!({ "ref": item, "state": "todo" }),
            )
            .await
            .unwrap();
        assert_eq!(recorded_state(&reopened), ("todo".into(), "ready".into()));
        let inverse = reopened.inverse.unwrap();
        assert_eq!(inverse.name, "update");
        let undone = tasks
            .invoke(VerbCall::bare(&Actor::Human, None), "update", inverse.input)
            .await
            .unwrap();
        assert_eq!(recorded_state(&undone), ("done".into(), "archived".into()));
    }

    /// A refused verb writes nothing: its transaction rolls back, and the
    /// refusal is the verb's own.
    #[tokio::test]
    async fn a_refused_verb_leaves_nothing() {
        let (tasks, db) = tasks().await;
        let err = tasks
            .invoke(
                VerbCall::bare(&Actor::Human, None),
                "create",
                json!({ "title": "t", "native_state": "nope" }),
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
