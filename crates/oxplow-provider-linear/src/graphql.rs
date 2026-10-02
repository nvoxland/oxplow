//! Linear's GraphQL API: one POST per operation, the API key as the
//! `Authorization` header, errors mapped onto the provider protocol's.

use oxplow_provider_protocol::ProtocolError;
use serde_json::{json, Value};

/// Where Linear's API lives; `LINEAR_API_URL` overrides it (the tests'
/// simulator).
pub const DEFAULT_URL: &str = "https://api.linear.app/graphql";

/// A named GraphQL document. The name is the document's operation name,
/// sent as `operationName`.
#[derive(Debug, Clone, Copy)]
pub struct Operation {
    pub name: &'static str,
    pub document: &'static str,
}

/// A client for one API key.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    url: String,
    key: String,
}

impl Client {
    pub fn new(url: &str, key: &str) -> Client {
        Client {
            http: reqwest::Client::new(),
            url: url.to_string(),
            key: key.to_string(),
        }
    }

    /// Run `op` with `variables`; its `data`.
    pub async fn run(&self, op: Operation, variables: Value) -> Result<Value, ProtocolError> {
        let body =
            json!({ "query": op.document, "operationName": op.name, "variables": variables });
        let response = self
            .http
            .post(&self.url)
            .header("Authorization", &self.key)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| ProtocolError::Internal(format!("Linear unreachable: {e}")))?;
        let status = response.status().as_u16();
        let retry_after = retry_after_ms(response.headers());
        let text = response
            .text()
            .await
            .map_err(|e| ProtocolError::Internal(format!("Linear's reply: {e}")))?;
        let reply: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let code = reply["errors"][0]["extensions"]["code"]
            .as_str()
            .unwrap_or_default();
        let message = reply["errors"][0]["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP {status}"));
        if status == 429 || code == "RATELIMITED" {
            return Err(ProtocolError::RateLimited {
                message: format!("Linear: {message}"),
                retry_after_ms: retry_after,
            });
        }
        if status == 401 || code == "AUTHENTICATION_ERROR" {
            return Err(ProtocolError::Auth(format!("Linear: {message}")));
        }
        // A bad argument (an issue that doesn't exist): the caller says
        // which of its inputs it was.
        if code == "INVALID_INPUT" {
            return Err(ProtocolError::InvalidInput {
                field: String::new(),
                message: format!("Linear: {message}"),
            });
        }
        if reply["errors"].as_array().is_some_and(|e| !e.is_empty())
            || !(200..300).contains(&status)
        {
            return Err(ProtocolError::Internal(format!(
                "Linear {}: {message}",
                op.name
            )));
        }
        Ok(reply["data"].clone())
    }
}

/// How long Linear asks to wait: `Retry-After` (seconds), else the
/// request budget's reset (`X-RateLimit-Requests-Reset`, epoch ms).
fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(secs) = header("retry-after").and_then(|v| v.trim().parse::<u64>().ok()) {
        return Some(secs * 1000);
    }
    let reset = header("x-ratelimit-requests-reset").and_then(|v| v.trim().parse::<u64>().ok())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(reset.saturating_sub(now))
}
