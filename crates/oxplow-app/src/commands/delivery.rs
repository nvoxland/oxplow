//! What a person does with an event a consumer couldn't take (a dead
//! letter, `v_event_dead_letter`): `oxplow.delivery.retry { id }` runs the
//! parked event through its consumer again, now (the letter is `retried`,
//! or stays `pending` with the new error and one more attempt), and
//! `oxplow.delivery.discard { id }` gives up on it (it stays visible as
//! `discarded`). `External`: a retry runs a consumer's handler, which may
//! reach outside oxplow, so the event pump owns the state; both are a
//! person's — or a lens acting for one — never an agent's, and discarding
//! is asked first.

use std::sync::Arc;

use oxplow_domain::{CommandError, Confirm, DomainError, Invokers};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::ops::Op;
use super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};

pub const RETRY: &str = "oxplow.delivery.retry";
pub const DISCARD: &str = "oxplow.delivery.discard";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LetterInput {
    /// The dead letter (`v_event_dead_letter.id`).
    pub id: i64,
}

fn refused(e: DomainError) -> CommandError {
    match e {
        DomainError::NotFound => invalid("/id", "no such dead letter"),
        DomainError::Invalid(m) => invalid("/id", m),
        other => failed(other),
    }
}

fn op(name: &'static str, services: ServicesSlot, discard: bool) -> Op {
    let op = Op::new(
        "deliveries.write",
        name,
        schema::<LetterInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: LetterInput = parse(input)?;
                let svc = services.get()?;
                let letter = if discard {
                    svc.event_pump.discard_dead_letter(input.id).await
                } else {
                    svc.event_pump.retry_dead_letter(input.id).await
                }
                .map_err(refused)?;
                Ok(HandlerOutput {
                    result: json!({
                        "id": letter.id,
                        "consumer": letter.consumer,
                        "event_seq": letter.event_seq,
                        "state": letter.state,
                        "attempts": letter.attempts,
                        "error": letter.error,
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .open_to(Invokers::NO_AGENT);
    if discard {
        op.confirm_at_least(Confirm::Destructive)
    } else {
        op
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        op("retry", services.clone(), false),
        op("discard", services, true),
    ]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    /// Park an event for `consumer`, as the pump would on a failure.
    async fn park(fx: &crate::test_fixtures::EffortFixture, consumer: &str) -> i64 {
        let consumer = consumer.to_string();
        fx.svc
            .db
            .transaction(move |tx| {
                let seq: i64 = tx
                    .query_row("SELECT max(seq) FROM event_log", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?;
                oxplow_db::event_log_store::dead_letter_tx(tx, &consumer, seq, "boom")?;
                tx.query_row("SELECT max(id) FROM event_dead_letter", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A person discards a letter (asked first); it stays visible as
    /// `discarded`. A retry names why it can't run: no such consumer.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_discards_and_retries_dead_letters() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let id = park(&fx, "no.such.consumer").await;
        let run = |name: &'static str, confirmed: bool, actor: Actor| {
            let fx = &fx;
            async move {
                fx.svc
                    .commands
                    .run(&actor, name, json!({ "id": id }), confirmed)
                    .await
            }
        };
        let err = run(super::RETRY, false, Actor::Human).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("no.such.consumer")),
            "{err:?}"
        );
        let ask = run(super::DISCARD, false, Actor::Human).await.unwrap_err();
        assert!(
            matches!(ask, CommandError::NeedsConfirmation { .. }),
            "{ask:?}"
        );
        let out = run(super::DISCARD, true, Actor::Human).await.unwrap();
        assert_eq!(out.result["state"], json!("discarded"));
        let gone = fx
            .svc
            .commands
            .run(&Actor::Human, super::RETRY, json!({ "id": 99999 }), false)
            .await
            .unwrap_err();
        assert!(matches!(gone, CommandError::Invalid { .. }), "{gone:?}");
    }

    /// An agent doesn't touch delivery: it is a person's, or a lens for one.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_may_not_retry_or_discard() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let id = park(&fx, "x").await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        for name in [super::RETRY, super::DISCARD] {
            let err = fx
                .svc
                .commands
                .run(&agent, name, json!({ "id": id }), true)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    CommandError::Denied { .. } | CommandError::Unknown { .. }
                ),
                "{name}: {err:?}"
            );
        }
    }
}
