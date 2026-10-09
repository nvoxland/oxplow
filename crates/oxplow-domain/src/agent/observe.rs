//! What core observes of an agent through its harness, in neutral shapes:
//! the answer to a hook ([`HookAnswer`], which the harness renders in its
//! own wire shape), the turns in a transcript ([`Turn`]), and the token
//! counts in a telemetry export ([`OtlpRecord`] → [`TokenReading`]).

use serde::{Deserialize, Serialize};

use crate::events::schema::TokenKind;
use crate::hook::HookKind;

/// What core answers a hook. The harness renders it
/// (`AgentHarness::render`). On the wire: `{ "kind": "ack" }`,
/// `{ "kind": "deny", "reason" }`, `{ "kind": "context", "event", "text" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookAnswer {
    /// Nothing to say: the call proceeds.
    Ack,
    /// Refuse the tool call (a PreToolUse), for `reason`.
    Deny { reason: String },
    /// Context for the agent after `event` (a PostToolUse or a
    /// UserPromptSubmit).
    Context { event: HookKind, text: String },
}

/// What a prompt hook (`UserPromptSubmit`) is, as its harness reads it.
/// On the wire, `{ "kind": "person", "text" }` or `{ "kind": "handback",
/// "subagent" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Prompt {
    /// A person's prompt: it opens a turn.
    Person { text: String },
    /// A background subagent handing its report back to its session
    /// (Claude Code posts it as a prompt): the subagent finished, and the
    /// agent goes on working — not a person's words.
    Handback {
        subagent: crate::agent::tool::Subagent,
    },
}

/// Summed usage across a chunk of transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageDelta {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub message_count: i64,
    /// The model of the last assistant message in the chunk.
    pub model: Option<String>,
}

impl UsageDelta {
    pub fn is_empty(&self) -> bool {
        self.message_count == 0
    }
}

/// One agent turn within a transcript chunk: the person's prompt that
/// opened it (when present) and the summed usage of the messages that
/// answered it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// The opening prompt, or `None` for a continuation with no fresh
    /// prompt at the head of the chunk.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub usage: UsageDelta,
}

impl Turn {
    /// Worth recording: it captured a prompt or usage.
    pub fn is_recordable(&self) -> bool {
        self.prompt.is_some() || !self.usage.is_empty()
    }
}

/// An OTLP attribute's value: on the wire, the JSON value itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Double(f64),
    Bool(bool),
}

/// Read through a JSON value, not an untagged enum: inside a record (an
/// internally tagged enum, which buffers its content) an untagged number
/// doesn't read back when serde_json keeps arbitrary-precision numbers,
/// which a workspace build turns on.
impl<'de> Deserialize<'de> for AttrValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::String(s) => Ok(AttrValue::Str(s)),
            serde_json::Value::Bool(b) => Ok(AttrValue::Bool(b)),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Ok(AttrValue::Int(i)),
                None => n.as_f64().map(AttrValue::Double).ok_or_else(|| {
                    D::Error::custom(format!("an attribute number out of range: {n}"))
                }),
            },
            other => Err(D::Error::custom(format!(
                "an attribute is a string, number or boolean, not {other}"
            ))),
        }
    }
}

/// An OTLP record's attributes, in order. On the wire, an object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attrs(pub Vec<(String, AttrValue)>);

impl Serialize for Attrs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Attrs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Attrs;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of attribute values")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Attrs, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, AttrValue>()? {
                    out.push((k, v));
                }
                Ok(Attrs(out))
            }
        }
        deserializer.deserialize_map(Visit)
    }
}

impl Attrs {
    fn get(&self, key: &str) -> Option<&AttrValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A string-valued attribute.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            AttrValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// An integer-valued attribute (an int, a double truncated, or a
    /// numeric string).
    pub fn int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            AttrValue::Int(i) => Some(*i),
            AttrValue::Double(d) => Some(*d as i64),
            AttrValue::Str(s) => s.parse().ok(),
            AttrValue::Bool(_) => None,
        }
    }
}

/// One record of an OTLP export, as decoded by core. On the wire,
/// `{ "kind": "point", "metric", "value", "attributes", "resource",
/// "time_unix_nano", "start_time_unix_nano" }` or `{ "kind": "log", … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OtlpRecord {
    /// A metric data point: a counter's or gauge's value, or a
    /// histogram's sum, truncated to an integer.
    Point {
        metric: String,
        value: i64,
        #[serde(default)]
        attributes: Attrs,
        #[serde(default)]
        resource: Attrs,
        #[serde(default)]
        time_unix_nano: u64,
        #[serde(default)]
        start_time_unix_nano: u64,
    },
    /// A log record; its time is when it happened, else when it was
    /// observed.
    Log {
        #[serde(default)]
        attributes: Attrs,
        #[serde(default)]
        resource: Attrs,
        #[serde(default)]
        time_unix_nano: u64,
    },
}

impl OtlpRecord {
    /// The model it names: the record's `model`, else the resource's,
    /// else `"unknown"`.
    pub fn model(&self) -> String {
        let (attributes, resource) = match self {
            OtlpRecord::Point {
                attributes,
                resource,
                ..
            }
            | OtlpRecord::Log {
                attributes,
                resource,
                ..
            } => (attributes, resource),
        };
        attributes
            .str("model")
            .or_else(|| resource.str("model"))
            .unwrap_or("unknown")
            .to_string()
    }
}

/// A token count a harness read out of a telemetry record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenReading {
    pub model: String,
    pub kind: TokenKind,
    pub value: i64,
    /// When it was measured (0: it didn't say).
    #[serde(default)]
    pub at_unix_nano: u64,
    /// Where its window starts: a delta point's start (the previous
    /// collection), a log record's own time (0: it didn't say).
    #[serde(default)]
    pub from_unix_nano: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_read_by_type_and_the_model_falls_back_to_the_resource() {
        let a = Attrs(vec![
            ("n".into(), AttrValue::Str("12".into())),
            ("d".into(), AttrValue::Double(3.9)),
            ("s".into(), AttrValue::Str("x".into())),
        ]);
        assert_eq!(
            (a.int("n"), a.int("d"), a.int("s")),
            (Some(12), Some(3), None)
        );
        assert_eq!(a.str("s"), Some("x"));
        let resource = Attrs(vec![("model".into(), AttrValue::Str("m".into()))]);
        let log = OtlpRecord::Log {
            attributes: a,
            resource,
            time_unix_nano: 0,
        };
        assert_eq!(log.model(), "m");
        let bare = OtlpRecord::Log {
            attributes: Attrs::default(),
            resource: Attrs::default(),
            time_unix_nano: 0,
        };
        assert_eq!(bare.model(), "unknown");
    }

    /// What a provider harness reads and answers: a record's attributes as
    /// an object, in order, each value the JSON value; an answer by `kind`.
    #[test]
    fn the_wire_shapes_round_trip() {
        let record = OtlpRecord::Point {
            metric: "m.tokens".into(),
            value: 7,
            attributes: Attrs(vec![
                ("type".into(), AttrValue::Str("input".into())),
                ("n".into(), AttrValue::Int(2)),
                ("x".into(), AttrValue::Double(1.5)),
                ("b".into(), AttrValue::Bool(true)),
            ]),
            resource: Attrs::default(),
            time_unix_nano: 9,
            start_time_unix_nano: 0,
        };
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["kind"], "point");
        assert_eq!(
            wire["attributes"],
            serde_json::json!({"type": "input", "n": 2, "x": 1.5, "b": true})
        );
        // Through text, which keeps the attributes' order whatever
        // serde_json's map is built with.
        let text = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<OtlpRecord>(&text).unwrap(), record);
        let deny = HookAnswer::Deny {
            reason: "no".into(),
        };
        let wire = serde_json::to_value(&deny).unwrap();
        assert_eq!(wire, serde_json::json!({"kind": "deny", "reason": "no"}));
        assert_eq!(serde_json::from_value::<HookAnswer>(wire).unwrap(), deny);
        let context = HookAnswer::Context {
            event: HookKind::PostToolUse,
            text: "t".into(),
        };
        assert_eq!(
            serde_json::from_value::<HookAnswer>(serde_json::to_value(&context).unwrap()).unwrap(),
            context
        );
    }
}
