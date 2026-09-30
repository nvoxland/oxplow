//! JSON-RPC 2.0 messages, one per line (NDJSON).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::errors::ErrorObject;

/// A request id. Providers and the host both number from 1.
pub type Id = u64;

/// One JSON-RPC 2.0 message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request {
        id: Id,
        method: String,
        params: Value,
    },
    Response {
        id: Id,
        result: Value,
    },
    Error {
        /// `None` when the failing message had no readable id.
        id: Option<Id>,
        error: ErrorObject,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a JSON-RPC 2.0 message: {0}")]
    Shape(String),
}

impl Message {
    /// The message as its wire object.
    pub fn to_value(&self) -> Value {
        match self {
            Message::Request { id, method, params } => {
                json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
            }
            Message::Response { id, result } => {
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Message::Error { id, error } => {
                json!({ "jsonrpc": "2.0", "id": id, "error": error })
            }
            Message::Notification { method, params } => {
                json!({ "jsonrpc": "2.0", "method": method, "params": params })
            }
        }
    }

    /// The message as one NDJSON line (newline included).
    pub fn to_line(&self) -> String {
        let mut line = self.to_value().to_string();
        line.push('\n');
        line
    }

    /// Read one message from its wire object.
    pub fn from_value(value: Value) -> Result<Message, CodecError> {
        let Value::Object(mut obj) = value else {
            return Err(CodecError::Shape("not an object".into()));
        };
        if obj.remove("jsonrpc") != Some(Value::String("2.0".into())) {
            return Err(CodecError::Shape("`jsonrpc` isn't \"2.0\"".into()));
        }
        let id = |obj: &Map<String, Value>| -> Result<Option<Id>, CodecError> {
            match obj.get("id") {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_u64().map(Some).ok_or_else(|| {
                    CodecError::Shape(format!("id {v} isn't a non-negative integer"))
                }),
            }
        };
        let method = obj
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let params = obj.remove("params").unwrap_or(Value::Null);
        match (method, id(&obj)?) {
            (Some(method), Some(id)) => Ok(Message::Request { id, method, params }),
            (Some(method), None) => Ok(Message::Notification { method, params }),
            (None, id) => {
                if let Some(error) = obj.remove("error") {
                    Ok(Message::Error {
                        id,
                        error: serde_json::from_value(error)?,
                    })
                } else if let (Some(result), Some(id)) = (obj.remove("result"), id) {
                    Ok(Message::Response { id, result })
                } else {
                    Err(CodecError::Shape(
                        "neither a request, a notification, a result nor an error".into(),
                    ))
                }
            }
        }
    }

    /// Read one message from an NDJSON line.
    pub fn from_line(line: &str) -> Result<Message, CodecError> {
        Message::from_value(serde_json::from_str(line.trim_end())?)
    }
}

/// The notifications either side may send about an in-flight request.
pub mod notify {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::Id;

    pub const CANCEL: &str = "$/cancel";
    pub const PROGRESS: &str = "$/progress";
    pub const RECORD: &str = "$/record";
    pub const STATE: &str = "$/state";

    /// `$/cancel` (host → provider): stop working on request `id`; it
    /// answers with a `Cancelled` error.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Cancel {
        pub id: Id,
    }

    /// `$/progress` (provider → host): how request `id` is going.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Progress {
        pub id: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub message: Option<String>,
        /// 0 to 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub fraction: Option<f64>,
    }

    /// `$/record` (provider → host): one row a `read` produced, streamed
    /// before the read's result.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Record {
        pub id: Id,
        /// The entity the row belongs to (a collector's declared entity).
        pub entity: String,
        pub row: Value,
    }

    /// `$/state` (provider → host): a checkpoint for request `id`'s read
    /// — the cursor the next read resumes from.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct State {
        pub id: Id,
        pub state: Value,
    }
}

impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Message::from_value(Value::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
