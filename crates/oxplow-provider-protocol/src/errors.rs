//! The protocol's errors: JSON-RPC's standard codes plus the ones a
//! provider reports about the world it talks to.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A JSON-RPC error object: `code`, `message` and `data`, nothing else
/// (like every wire type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// The instance isn't configured (no `check`ed handle, or its config is
/// incomplete).
pub const NOT_CONFIGURED: i64 = -32001;
/// The provider's credentials were refused; `data.credential` names the
/// one, when the provider knows which.
pub const AUTH: i64 = -32002;
/// The provider's upstream is rate limiting; `data.retry_after_ms` says
/// when to try again.
pub const RATE_LIMITED: i64 = -32003;
/// An input field the provider rejects; `data.field` names it.
pub const INVALID_INPUT: i64 = -32004;
/// The request was cancelled (`$/cancel`).
pub const CANCELLED: i64 = -32800;

/// A protocol error, typed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProtocolError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("no method `{0}`")]
    MethodNotFound(String),
    #[error("invalid params: {0}")]
    InvalidParams(String),
    #[error("{0}")]
    Internal(String),
    #[error("not configured: {0}")]
    NotConfigured(String),
    /// Its credentials were refused — `credential` the one the service
    /// refused, when the provider knows (the host renews that one alone).
    #[error("authentication failed: {message}")]
    Auth {
        message: String,
        credential: Option<String>,
    },
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after_ms: Option<u64>,
    },
    #[error("invalid input at `{field}`: {message}")]
    InvalidInput { field: String, message: String },
    #[error("cancelled")]
    Cancelled,
    /// A code this side doesn't know.
    #[error("error {code}: {message}")]
    Other { code: i64, message: String },
}

impl From<&ProtocolError> for ErrorObject {
    fn from(e: &ProtocolError) -> Self {
        let (code, data) = match e {
            ProtocolError::Parse(_) => (PARSE_ERROR, None),
            ProtocolError::InvalidRequest(_) => (INVALID_REQUEST, None),
            ProtocolError::MethodNotFound(_) => (METHOD_NOT_FOUND, None),
            ProtocolError::InvalidParams(_) => (INVALID_PARAMS, None),
            ProtocolError::Internal(_) => (INTERNAL_ERROR, None),
            ProtocolError::NotConfigured(_) => (NOT_CONFIGURED, None),
            ProtocolError::Auth { credential, .. } => (
                AUTH,
                credential.as_ref().map(|c| json!({ "credential": c })),
            ),
            ProtocolError::RateLimited { retry_after_ms, .. } => (
                RATE_LIMITED,
                retry_after_ms.map(|ms| json!({ "retry_after_ms": ms })),
            ),
            ProtocolError::InvalidInput { field, .. } => {
                (INVALID_INPUT, Some(json!({ "field": field })))
            }
            ProtocolError::Cancelled => (CANCELLED, None),
            ProtocolError::Other { code, .. } => (*code, None),
        };
        let message = match e {
            ProtocolError::InvalidInput { message, .. }
            | ProtocolError::RateLimited { message, .. }
            | ProtocolError::Auth { message, .. } => message.clone(),
            other => other.to_string(),
        };
        ErrorObject {
            code,
            message,
            data,
        }
    }
}

impl From<ErrorObject> for ProtocolError {
    fn from(e: ErrorObject) -> Self {
        let field = |key: &str| e.data.as_ref().and_then(|d| d.get(key)).cloned();
        match e.code {
            PARSE_ERROR => ProtocolError::Parse(e.message),
            INVALID_REQUEST => ProtocolError::InvalidRequest(e.message),
            METHOD_NOT_FOUND => ProtocolError::MethodNotFound(e.message),
            INVALID_PARAMS => ProtocolError::InvalidParams(e.message),
            INTERNAL_ERROR => ProtocolError::Internal(e.message),
            NOT_CONFIGURED => ProtocolError::NotConfigured(e.message),
            AUTH => ProtocolError::Auth {
                credential: field("credential").and_then(|v| v.as_str().map(str::to_string)),
                message: e.message,
            },
            RATE_LIMITED => ProtocolError::RateLimited {
                retry_after_ms: field("retry_after_ms").and_then(|v| v.as_u64()),
                message: e.message,
            },
            INVALID_INPUT => ProtocolError::InvalidInput {
                field: field("field")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                message: e.message,
            },
            CANCELLED => ProtocolError::Cancelled,
            code => ProtocolError::Other {
                code,
                message: e.message,
            },
        }
    }
}
