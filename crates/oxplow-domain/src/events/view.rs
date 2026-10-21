//! The event envelope an extension sees (`.context/extensions.md` "The
//! event envelope"): one projection of a logged event, the same whichever
//! path hands it over — a collector's `input.event`, an effect's
//! `x["event"]`, an effort policy's event, a provider's `react { event }`,
//! an `on:` exec collector's stdin — and built the same way from a test
//! fixture. Every field is always present; what an event doesn't have is
//! `null`, never made up. The payload is always at its type's newest
//! registered version, so a script is written against one shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Anchors, StoredEvent};
use crate::ids::{AgentSessionId, EffortId, StreamId, ThreadId};
use crate::time::Timestamp;
use crate::vocabulary::Vocabulary;
use crate::DomainError;

/// One event as an extension sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventView {
    /// The event's id; `null` for a fixture that names none.
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    /// The newest registered version of `type`: what `payload` is at.
    pub v: u32,
    /// Its place in the log; `null` for a fixture that gives none.
    pub seq: Option<i64>,
    pub at: Option<Timestamp>,
    /// What emitted it (`agent:thr3`, `human`, `collector:<ext>/<id>`);
    /// `fixture` for a test's.
    pub source: String,
    /// The canonical refs it's about.
    pub subject: Vec<String>,
    /// Every anchor, `null` when the event has none.
    pub anchors: Anchors,
    pub payload: Value,
    /// The id of the event that caused it.
    pub cause: Option<String>,
}

impl EventView {
    /// A logged event, carried to its type's newest version.
    pub fn of(event: &StoredEvent, vocabulary: &Vocabulary) -> Result<Self, DomainError> {
        let event = at_latest(vocabulary, event)?;
        let env = event.envelope;
        Ok(Self {
            id: Some(env.id.0),
            event_type: env.event_type,
            v: env.v,
            seq: Some(event.seq),
            at: Some(env.at),
            source: env.source,
            subject: env.subject,
            anchors: env.anchors,
            payload: env.payload,
            cause: env.cause.map(|c| c.0),
        })
    }

    /// A test fixture's event (`event:` in an example): its `type` must be
    /// registered, its payload valid at the `v` it gives (the newest when
    /// it gives none) and carried to the newest from there, its subjects
    /// canonical refs and its anchors typed ids. What it doesn't give is
    /// `null`; its `source` is `fixture` unless it says.
    pub fn fixture(given: &Value, vocabulary: &Vocabulary) -> Result<Self, DomainError> {
        let invalid = |m: String| DomainError::Invalid(format!("event: {m}"));
        let f: FixtureEvent = serde_path_to_error::deserialize(given).map_err(|e| {
            match e.path().to_string().as_str() {
                "." => invalid(e.inner().to_string()),
                path => invalid(format!("`{path}`: {}", e.inner())),
            }
        })?;
        let latest = vocabulary
            .latest(&f.event_type)
            .ok_or_else(|| invalid(format!("`{}` is not a registered event type", f.event_type)))?;
        let from_v = f.v.unwrap_or(latest);
        let payload = f
            .payload
            .unwrap_or_else(|| Value::Object(Default::default()));
        if from_v <= latest {
            vocabulary
                .validate(&f.event_type, from_v, &payload)
                .map_err(|e| invalid(e.message()))?;
        }
        let (v, payload) = vocabulary
            .upcast_to_latest(&f.event_type, from_v, payload)
            .map_err(|e| invalid(e.message()))?;
        let subject = f.subject.unwrap_or_default();
        for s in &subject {
            crate::uri::validate_ref(&vocabulary.kinds, s)
                .map_err(|e| invalid(format!("subject `{s}`: {}", e.message())))?;
        }
        Ok(Self {
            id: f.id,
            event_type: f.event_type,
            v,
            seq: f.seq,
            at: f.at,
            source: f.source.unwrap_or_else(|| "fixture".into()),
            subject,
            anchors: f.anchors.map(Anchors::from).unwrap_or_default(),
            payload,
            cause: f.cause,
        })
    }

    /// As JSON: every field, `null` where absent.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("an event view serializes")
    }
}

/// The event as a consumer reads it: carried to the newest registered
/// version of its type (P3.1), so a consumer is written against one shape
/// and rows logged at an older version still reach it. A row that can't
/// be upcast is an error (the pump parks it as a dead letter). A type the
/// running vocabulary doesn't know (its extension was removed) passes
/// through as logged: there is no newer shape to carry it to.
pub fn at_latest(vocabulary: &Vocabulary, event: &StoredEvent) -> Result<StoredEvent, DomainError> {
    let env = &event.envelope;
    match vocabulary.latest(&env.event_type) {
        None => return Ok(event.clone()),
        Some(latest) if latest == env.v => return Ok(event.clone()),
        Some(_) => {}
    }
    let (v, payload) = vocabulary.upcast_to_latest(&env.event_type, env.v, env.payload.clone())?;
    let mut out = event.clone();
    out.envelope.v = v;
    out.envelope.payload = payload;
    Ok(out)
}

/// An `event:` as a fixture writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEvent {
    #[serde(rename = "type")]
    event_type: String,
    v: Option<u32>,
    payload: Option<Value>,
    subject: Option<Vec<String>>,
    anchors: Option<FixtureAnchors>,
    id: Option<String>,
    seq: Option<i64>,
    at: Option<Timestamp>,
    source: Option<String>,
    cause: Option<String>,
}

/// A fixture's anchors: [`Anchors`], refusing a key it doesn't have.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureAnchors {
    stream_id: Option<StreamId>,
    thread_id: Option<ThreadId>,
    effort_id: Option<EffortId>,
    turn_id: Option<i64>,
    agent_session_id: Option<AgentSessionId>,
    snapshot_id: Option<i64>,
}

impl From<FixtureAnchors> for Anchors {
    fn from(a: FixtureAnchors) -> Self {
        Anchors {
            stream_id: a.stream_id,
            thread_id: a.thread_id,
            effort_id: a.effort_id,
            turn_id: a.turn_id,
            agent_session_id: a.agent_session_id,
            snapshot_id: a.snapshot_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::events::schema::{DeclaredEventType, EventSchemaRegistry};
    use crate::events::{Envelope, EventId};
    use crate::uri::types::core_types;

    /// Core's types plus `acme.thing`, v1 `{ n }` and v2 `{ count }`, its
    /// upcast renaming the field.
    pub(crate) fn vocabulary() -> Vocabulary {
        let mut events = EventSchemaRegistry::core();
        let schema = |field: &str| {
            json!({ "type": "object", "required": [field], "additionalProperties": false,
                    "properties": { field: { "type": "integer" } } })
        };
        events
            .register_declared(
                "acme",
                DeclaredEventType {
                    event_type: "acme.thing".into(),
                    v: 1,
                    schema: schema("n"),
                    summary: "a thing".into(),
                    upcast: None,
                },
            )
            .unwrap();
        events
            .register_declared(
                "acme",
                DeclaredEventType {
                    event_type: "acme.thing".into(),
                    v: 2,
                    schema: schema("count"),
                    summary: "a thing, counted".into(),
                    upcast: Some(Arc::new(|_, p| Ok(json!({ "count": p["n"] })))),
                },
            )
            .unwrap();
        Vocabulary::new(events, core_types())
    }

    fn stored(v: u32, payload: Value) -> StoredEvent {
        let mut env = Envelope::new("acme.thing", v, "collector:acme/c", payload)
            .unwrap()
            .with_anchors(Anchors {
                thread_id: Some(ThreadId::new(3)),
                turn_id: Some(7),
                ..Default::default()
            })
            .with_subject(["work_item:oxplow:tsk4"])
            .with_cause(EventId("01990000-0000-7000-8000-000000000001".into()));
        env.at = Timestamp::parse("2026-10-09T12:00:00.000000Z").unwrap();
        StoredEvent {
            seq: 42,
            envelope: env,
            payload_expired_at: None,
        }
    }

    /// A logged event's envelope has every key, `null` where the event has
    /// nothing, and its payload carried to the newest version.
    #[test]
    fn a_logged_event_is_every_field_at_its_newest_version() {
        let event = stored(1, json!({ "n": 5 }));
        let view = EventView::of(&event, &vocabulary()).unwrap().to_json();
        assert_eq!(
            view,
            json!({
                "id": event.envelope.id.0,
                "type": "acme.thing",
                "v": 2,
                "seq": 42,
                "at": "2026-10-09T12:00:00.000000Z",
                "source": "collector:acme/c",
                "subject": ["work_item:oxplow:tsk4"],
                "anchors": {
                    "stream_id": null, "thread_id": "thr3", "effort_id": null,
                    "turn_id": 7, "agent_session_id": null, "snapshot_id": null
                },
                "payload": { "count": 5 },
                "cause": "01990000-0000-7000-8000-000000000001",
            })
        );
    }

    /// A fixture gives what it means to and gets `null` for the rest; a
    /// fixture at an older version is carried like a logged event, so the
    /// same facts make the same envelope either way.
    #[test]
    fn a_fixture_is_the_same_envelope_as_the_logged_event() {
        let vocab = vocabulary();
        let event = stored(1, json!({ "n": 5 }));
        let given = json!({
            "id": event.envelope.id.0, "type": "acme.thing", "v": 1, "seq": 42,
            "at": "2026-10-09T12:00:00.000000Z", "source": "collector:acme/c",
            "subject": ["work_item:oxplow:tsk4"],
            "anchors": { "thread_id": "thr3", "turn_id": 7 },
            "payload": { "n": 5 },
            "cause": "01990000-0000-7000-8000-000000000001",
        });
        assert_eq!(
            EventView::fixture(&given, &vocab).unwrap(),
            EventView::of(&event, &vocab).unwrap()
        );
        let bare = EventView::fixture(
            &json!({ "type": "acme.thing", "payload": { "count": 1 } }),
            &vocab,
        )
        .unwrap()
        .to_json();
        assert_eq!(bare["v"], 2, "no `v` is the newest");
        for absent in ["id", "seq", "at", "cause"] {
            assert_eq!(bare[absent], Value::Null, "{absent}");
        }
        assert_eq!(bare["source"], "fixture");
        assert_eq!(bare["anchors"]["thread_id"], Value::Null);
    }

    /// A fixture that couldn't happen is an error naming what's wrong.
    #[test]
    fn a_fixture_that_couldnt_happen_says_why() {
        let vocab = vocabulary();
        for (given, needle) in [
            (
                json!({ "type": "acme.thing", "v": 3, "payload": {} }),
                "newer than the registered v2",
            ),
            (
                json!({ "type": "acme.nothing" }),
                "`acme.nothing` is not a registered event type",
            ),
            (
                json!({ "type": "acme.thing", "v": 1, "payload": { "count": 1 } }),
                "`acme.thing@1` payload invalid",
            ),
            (
                json!({ "type": "acme.thing", "payload": { "count": 1 }, "anchors": { "turn_id": "seven" } }),
                "turn_id",
            ),
            (
                json!({ "type": "acme.thing", "payload": { "count": 1 }, "anchors": { "thread_id": "eff1" } }),
                "thread_id",
            ),
            (
                json!({ "type": "acme.thing", "payload": { "count": 1 }, "anchors": { "thread": "thr1" } }),
                "unknown field `thread`",
            ),
            (
                json!({ "type": "acme.thing", "payload": { "count": 1 }, "subjects": [] }),
                "unknown field `subjects`",
            ),
            (
                json!({ "type": "acme.thing", "payload": { "count": 1 }, "subject": ["nope"] }),
                "subject `nope`",
            ),
        ] {
            let err = EventView::fixture(&given, &vocab).unwrap_err().to_string();
            assert!(err.contains(needle), "{given}: {err}");
            assert!(err.contains("event: "), "{err}");
        }
    }

    /// A type the running vocabulary doesn't know passes as logged.
    #[test]
    fn an_unknown_type_passes_as_logged() {
        let event = stored(1, json!({ "n": 5 }));
        let view = EventView::of(&event, &Vocabulary::core()).unwrap();
        assert_eq!((view.v, view.payload), (1, json!({ "n": 5 })));
    }
}
