//! The event log's envelope (`.context/data-model.md` "event_log").
//!
//! State tables hold current truth; the event log records activity and
//! state changes and is written in the **same transaction** as the
//! change (the outbox pattern), so the two never disagree. This module
//! is the envelope only — pure data, no IO. Persistence lives in
//! `oxplow_db::event_log_store`; the per-type payload schemas live in
//! [`schema`]; the delivery pump is a later P1 step.

pub mod retention;
pub mod schema;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::ids::{EffortId, StreamId, ThreadId};
use crate::time::Timestamp;
use crate::DomainError;

/// An event's public identity: a UUIDv7 in its canonical text form, so
/// ids sort by creation time. `seq` (the log's insert order) is the
/// delivery order; `id` is what other events and the audit log point at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct EventId(pub String);

impl EventId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where an event sits in oxplow's timeline. The engine fills these
/// from context; every field is optional because not every event has a
/// thread (a snapshot) or an effort (a stream-level config change).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Anchors {
    pub stream_id: Option<StreamId>,
    pub thread_id: Option<ThreadId>,
    pub effort_id: Option<EffortId>,
    /// `agent_turn.id`.
    pub turn_id: Option<i64>,
    /// `snapshot.id`.
    pub snapshot_id: Option<i64>,
}

/// One event as written to the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Envelope {
    pub id: EventId,
    /// The event's name, `namespace.name[.name]` (`work_item.transitioned`,
    /// `agent.tool.finished`). Plugins emit only under their own namespace.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The schema version of `event_type`; the payload validates against
    /// `type@v`.
    pub v: u32,
    pub at: Timestamp,
    /// What emitted it: `agent:thr3`, `human`, `system:snapshot_capture`.
    pub source: String,
    pub anchors: Anchors,
    /// The canonical refs (`.context/refs.md`) the event is about.
    pub subject: Vec<String>,
    /// Validated against the type's JSON Schema on append (P1.5). Large or
    /// sensitive content is never inline: it is stored by content hash and
    /// the payload carries the hash.
    #[specta(type = specta_typescript::Unknown)]
    pub payload: Value,
    pub payload_hash: Option<String>,
    /// The event that caused this one.
    pub cause: Option<EventId>,
    /// A stable key the emitter derives from the action (`work_item:tsk4:
    /// transition:eff9`), so an at-least-once producer can't log the same
    /// occurrence twice: the second append fails with `Constraint`.
    pub dedupe_key: Option<String>,
}

impl Envelope {
    /// A fresh envelope: new id, `at = now`, no anchors, empty subject.
    /// `event_type` must be a valid type name.
    pub fn new(
        event_type: impl Into<String>,
        v: u32,
        source: impl Into<String>,
        payload: Value,
    ) -> Result<Self, DomainError> {
        let event_type = event_type.into();
        validate_type_name(&event_type)?;
        Ok(Self {
            id: EventId::generate(),
            event_type,
            v,
            at: Timestamp::now(),
            source: source.into(),
            anchors: Anchors::default(),
            subject: Vec::new(),
            payload,
            payload_hash: None,
            cause: None,
            dedupe_key: None,
        })
    }

    pub fn with_anchors(mut self, anchors: Anchors) -> Self {
        self.anchors = anchors;
        self
    }

    pub fn with_subject(mut self, refs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.subject = refs.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_cause(mut self, cause: EventId) -> Self {
        self.cause = Some(cause);
        self
    }

    pub fn with_dedupe_key(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = Some(key.into());
        self
    }

    /// [`Self::with_dedupe_key`] when the producer has a key.
    pub fn with_dedupe_key_opt(mut self, key: Option<String>) -> Self {
        self.dedupe_key = key;
        self
    }

    /// The namespace: the text before the first `.`.
    pub fn namespace(&self) -> &str {
        self.event_type.split('.').next().unwrap_or("")
    }
}

/// An envelope as read back from the log, with its position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StoredEvent {
    /// Global insert order; the delivery order and what checkpoints hold.
    pub seq: i64,
    #[serde(flatten)]
    pub envelope: Envelope,
    /// When retention replaced the payload with `{}` (the envelope stays).
    /// An expired event is history only: the pump skips it and nothing
    /// derives state from it.
    pub payload_expired_at: Option<Timestamp>,
}

impl StoredEvent {
    pub fn payload_expired(&self) -> bool {
        self.payload_expired_at.is_some()
    }
}

/// A type name is `segment(.segment)+` with snake_case segments: at
/// least a namespace and a name.
pub fn validate_type_name(name: &str) -> Result<(), DomainError> {
    let segments: Vec<&str> = name.split('.').collect();
    let ok = segments.len() >= 2
        && segments.iter().all(|s| {
            let mut chars = s.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
    if ok {
        Ok(())
    } else {
        Err(DomainError::Invalid(format!(
            "event type `{name}` must be `namespace.name` in snake_case (e.g. `work_item.transitioned`)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_creation_time() {
        let a = EventId::generate();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = EventId::generate();
        assert!(a.0 < b.0, "{a} should sort before {b}");
        assert_eq!(a.0.len(), 36);
    }

    #[test]
    fn type_names_are_namespaced_snake_case() {
        assert!(validate_type_name("work_item.transitioned").is_ok());
        assert!(validate_type_name("agent.tool.finished").is_ok());
        assert!(validate_type_name("acme_review.decision2.made").is_ok());
        for bad in [
            "transitioned",
            "Work.Item",
            "work-item.done",
            "a..b",
            ".a",
            "a.",
            "",
        ] {
            assert!(
                validate_type_name(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(Envelope::new("nope", 1, "test", Value::Null).is_err());
    }

    #[test]
    fn envelope_serializes_with_type_and_flattened_seq() {
        let env = Envelope::new(
            "work_item.transitioned",
            1,
            "human",
            serde_json::json!({"to": "done"}),
        )
        .unwrap()
        .with_subject(["work_item:oxplow:tsk4"])
        .with_dedupe_key("work_item:tsk4:transition:1");
        assert_eq!(env.namespace(), "work_item");
        let json = serde_json::to_value(StoredEvent {
            seq: 7,
            envelope: env.clone(),
            payload_expired_at: None,
        })
        .unwrap();
        assert_eq!(json["seq"], 7);
        assert_eq!(json["type"], "work_item.transitioned");
        assert_eq!(json["id"], env.id.0);
        assert_eq!(json["subject"][0], "work_item:oxplow:tsk4");
        let back: StoredEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back.envelope, env);
    }
}
