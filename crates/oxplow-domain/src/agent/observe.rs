//! What core observes of an agent through its harness, in neutral shapes:
//! the answer to a hook ([`HookAnswer`], which the harness renders in its
//! own wire shape), the turns in a transcript ([`Turn`]), and the token
//! counts in a telemetry export ([`OtlpRecord`] → [`TokenReading`]).

use crate::events::schema::TokenKind;
use crate::hook::HookKind;

/// What core answers a hook. The harness renders it
/// (`AgentHarness::render`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookAnswer {
    /// Nothing to say: the call proceeds.
    Ack,
    /// Refuse the tool call (a PreToolUse), for `reason`.
    Deny { reason: String },
    /// Context for the agent after `event` (a PostToolUse or a
    /// UserPromptSubmit).
    Context { event: HookKind, text: String },
}

/// Summed usage across a chunk of transcript.
#[derive(Debug, Clone, Default, PartialEq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    /// The opening prompt, or `None` for a continuation with no fresh
    /// prompt at the head of the chunk.
    pub prompt: Option<String>,
    pub usage: UsageDelta,
}

impl Turn {
    /// Worth recording: it captured a prompt or usage.
    pub fn is_recordable(&self) -> bool {
        self.prompt.is_some() || !self.usage.is_empty()
    }
}

/// An OTLP attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Double(f64),
    Bool(bool),
}

/// An OTLP record's attributes, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attrs(pub Vec<(String, AttrValue)>);

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

/// One record of an OTLP export, as decoded by core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OtlpRecord<'a> {
    /// A metric data point: a counter's or gauge's value, or a
    /// histogram's sum, truncated to an integer.
    Point {
        metric: &'a str,
        value: i64,
        attributes: &'a Attrs,
        resource: &'a Attrs,
        time_unix_nano: u64,
        start_time_unix_nano: u64,
    },
    /// A log record; its time is when it happened, else when it was
    /// observed.
    Log {
        attributes: &'a Attrs,
        resource: &'a Attrs,
        time_unix_nano: u64,
    },
}

impl OtlpRecord<'_> {
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
#[derive(Debug, Clone, PartialEq)]
pub struct TokenReading {
    pub model: String,
    pub kind: TokenKind,
    pub value: i64,
    /// When it was measured (0: it didn't say).
    pub at_unix_nano: u64,
    /// Where its window starts: a delta point's start (the previous
    /// collection), a log record's own time (0: it didn't say).
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
            attributes: &a,
            resource: &resource,
            time_unix_nano: 0,
        };
        assert_eq!(log.model(), "m");
        let none = Attrs::default();
        let bare = OtlpRecord::Log {
            attributes: &none,
            resource: &none,
            time_unix_nano: 0,
        };
        assert_eq!(bare.model(), "unknown");
    }
}
