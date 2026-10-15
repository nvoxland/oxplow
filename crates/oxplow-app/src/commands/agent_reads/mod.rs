//! The agent's reads as operations of read scopes (`records.read`,
//! `sql.read`, `search.read`, `pages.read`, `agent_text.read`,
//! `lenses.read`, `lsp.read`, `vcs.read`, `extensions.check`,
//! `collectors.preview`, `ai.compute`, `knowledge.read`, `changes.read`).
//! Each is `Access::Read` on the daemon, with an `External` handler that
//! reads live state through `Services`; `extensions/oxplow-foundation`
//! declares the commands over them (`oxplow.records.read`,
//! `oxplow.sql.query`, …), so an agent reaches each through
//! `run_command` and a person or lens through the bus.
//!
//! An agent's call runs as its `Actor::Agent`: a read that acts on a
//! stream takes the one it names, else the actor's own
//! ([`actor_stream`]).

use oxplow_domain::{Actor, CommandError};

use super::ops::Op;
use super::util::{failed, ServicesSlot};
use crate::Services;

pub mod ai;
pub mod changes;
pub mod collectors;
pub mod extensions;
pub mod ids;
pub mod knowledge;
pub mod lens;
pub mod lsp;
pub mod pages;
pub mod records;
pub mod search;
pub mod skill;
pub mod sql;
pub mod vcs;

/// Every operation of the agent's reads.
pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let mut ops = Vec::new();
    ops.extend(records::ops(services.clone()));
    ops.extend(sql::ops(services.clone()));
    ops.extend(search::ops(services.clone()));
    ops.extend(pages::ops(services.clone()));
    ops.extend(skill::ops(services.clone()));
    ops.extend(lens::ops(services.clone()));
    ops.extend(lsp::ops(services.clone()));
    ops.extend(vcs::ops(services.clone()));
    ops.extend(extensions::ops(services.clone()));
    ops.extend(collectors::ops(services.clone()));
    ops.extend(ai::ops(services.clone()));
    ops.extend(knowledge::ops(services.clone()));
    ops.extend(changes::ops(services.clone()));
    ops
}

/// The stream a stream-scoped read acts on when the call names none: the
/// actor's own (its stream, else its thread's); `None` for an actor with
/// neither (a person, an anonymous connection).
pub async fn actor_stream(svc: &Services, actor: &Actor) -> Result<Option<String>, CommandError> {
    use oxplow_domain::stores::ThreadStore as _;
    let Actor::Agent {
        thread_id,
        stream_id,
        ..
    } = actor
    else {
        return Ok(None);
    };
    if let Some(stream) = stream_id {
        return Ok(Some(stream.to_string()));
    }
    let Some(thread) = thread_id else {
        return Ok(None);
    };
    let found = svc.thread_store.get(thread).await.map_err(failed)?;
    Ok(found.map(|t| t.stream_id.to_string()))
}

/// [`actor_stream`], or what the call `given`.
pub async fn stream_or_actors(
    svc: &Services,
    actor: &Actor,
    given: Option<String>,
) -> Result<Option<String>, CommandError> {
    match given {
        Some(stream) => Ok(Some(stream)),
        None => actor_stream(svc, actor).await,
    }
}

/// A stream id (`str2`) a call named, refused at `field` when it isn't
/// one — a thread's or any other id is the usual slip.
pub fn check_stream_id(field: &str, raw: &str) -> Result<(), CommandError> {
    if oxplow_domain::StreamId::try_from_str(raw).is_some() {
        return Ok(());
    }
    let looks_like = raw
        .parse::<oxplow_domain::AnyId>()
        .map(|id| format!(", which looks like a {}", id.kind.label()))
        .unwrap_or_default();
    Err(super::util::invalid(
        field,
        format!("expected a stream id (str…), got `{raw}`{looks_like}"),
    ))
}
