//! How an event came to be, walked back through its causes: what the
//! loop guards of the two kinds of event-triggered code read
//! (`effect_triggers`, `collector_triggers`).
//!
//! A **hop** is one reaction: an effect's run (its `command.executed`,
//! from `effect:<extension>/<id>`) or a collector's run for an event (its
//! `collector.synced` with `trigger: on`). An event that [`MAX_CHAIN`]
//! hops already led to is reacted to by nothing; neither is an event a
//! reaction's own run led to.

use oxplow_db::Database;
use oxplow_domain::events::StoredEvent;
use oxplow_domain::DomainError;

/// The reactions that may follow one another before the next is refused.
pub const MAX_CHAIN: usize = 4;

/// How far back a chain is followed: more than the guard needs.
const MAX_WALK: usize = 64;

/// What led to an event, as far as a guard asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lineage {
    /// An event from `source` is among its causes (or is it): the asking
    /// reaction's own run led to it.
    pub own: bool,
    /// The hops that led to it (counted up to where `own` was found).
    pub depth: usize,
}

impl Lineage {
    /// Why a reaction to the event is refused, if it is.
    pub fn refusal(&self) -> Option<String> {
        if self.own {
            Some("loop guard: its own run led to this event".into())
        } else if self.depth >= MAX_CHAIN {
            Some(format!(
                "loop guard: {MAX_CHAIN} reactions already led to this event"
            ))
        } else {
            None
        }
    }
}

/// Walk `event`'s causes for a reaction whose own events carry `source`
/// (`effect:<extension>/<id>`, `collector:<owner>/<id>`).
pub async fn lineage(
    db: &Database,
    event: &StoredEvent,
    source: &str,
) -> Result<Lineage, DomainError> {
    let (first, source) = (event.clone(), source.to_string());
    db.read(move |tx| lineage_tx(tx, first, &source)).await
}

/// [`lineage`] inside a transaction already open (a backfill's plan
/// reads it for each candidate event).
pub fn lineage_tx(
    tx: &rusqlite::Connection,
    event: StoredEvent,
    source: &str,
) -> Result<Lineage, DomainError> {
    let (mut own, mut depth) = (false, 0usize);
    let mut current = Some(event);
    for _ in 0..MAX_WALK {
        let Some(e) = current.take() else { break };
        let env = &e.envelope;
        if env.source == source {
            own = true;
            break;
        }
        let effect_run = env.event_type == "command.executed" && env.source.starts_with("effect:");
        let collector_run = env.event_type == "collector.synced" && env.payload["trigger"] == "on";
        if effect_run || collector_run {
            depth += 1;
        }
        current = match &env.cause {
            Some(cause) => oxplow_db::event_log_store::get_tx(tx, cause)?,
            None => None,
        };
    }
    Ok(Lineage { own, depth })
}
