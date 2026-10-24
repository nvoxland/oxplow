//! Rig's errors as oxplow's [`AiError`], with the instance's key taken out
//! of anything a provider echoed and of every transport message.

use oxplow_ai::client::AiError;
use rig_core::error::ProviderError;

/// The start of an error body an [`AiError::Http`] keeps.
const BODY_CHARS: usize = 300;

/// Whether `c` can be part of a key: a credential is a token, so it is
/// only taken out where it stands whole.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `text` with `key` replaced wherever the characters around it aren't key
/// characters (the ends of the text count as boundaries). A short key
/// doesn't mangle the words that contain it.
pub fn redact(text: &str, key: Option<&str>) -> String {
    let Some(key) = key.filter(|key| !key.is_empty()) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    while let Some(found) = text[rest..].find(key) {
        let (start, end) = (rest + found, rest + found + key.len());
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_none_or(|c| !is_key_char(c)) && after.is_none_or(|c| !is_key_char(c)) {
            out.push_str(&text[rest..start]);
            out.push_str("[redacted]");
            rest = end;
        } else {
            // Not a whole token: keep its first character and look on.
            let step = text[start..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[rest..start + step]);
            rest = start + step;
        }
    }
    out.push_str(&text[rest..]);
    out
}

/// A transport failure's message without the URL it was sent to (a query
/// can hold a key) and without the key.
fn transport_message(error: &rig_core::http_client::Error, key: Option<&str>) -> String {
    let mut message = error.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(e) = source {
        if let Some(url) = e
            .downcast_ref::<rig_reqwest::reqwest::Error>()
            .and_then(|e| e.url())
        {
            message = message.replace(&format!(" ({url})"), "");
            message = message.replace(url.as_str(), "");
        }
        source = e.source();
    }
    redact(&message, key)
}

/// `error`, from a call by the provider `provider` made with `key`.
pub fn ai_error(provider: &str, key: Option<&str>, error: ProviderError) -> AiError {
    let id = || provider.to_string();
    let detail = |text: &str| redact(text, key);
    match error {
        ProviderError::InvalidAuthentication(_) => AiError::Auth { provider: id() },
        ProviderError::ProviderResponse(response) => match response.status.map(|s| s.as_u16()) {
            Some(401 | 403) => AiError::Auth { provider: id() },
            Some(429) => AiError::RateLimited { provider: id() },
            Some(status) if !(200..=299).contains(&status) => AiError::Http {
                provider: id(),
                status,
                body: detail(&response.body).chars().take(BODY_CHARS).collect(),
            },
            // A 2xx error envelope, or a stream's error event.
            _ => AiError::BadResponse {
                provider: id(),
                detail: detail(&response.body).chars().take(BODY_CHARS).collect(),
            },
        },
        ProviderError::Http(error) => AiError::Transport {
            provider: id(),
            message: transport_message(&error, key),
        },
        ProviderError::Truncated => AiError::BadResponse {
            provider: id(),
            detail: "the reply ended before the provider finished it".into(),
        },
        error @ (ProviderError::Request(_)
        | ProviderError::UnsupportedOption(_)
        | ProviderError::Url(_)) => AiError::BadRequest {
            provider: id(),
            detail: detail(&error.to_string()),
        },
        error => AiError::BadResponse {
            provider: id(),
            detail: detail(&error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key is taken out as a whole token, wherever it is echoed; a
    /// short key leaves the words that hold it alone.
    #[test]
    fn a_key_is_redacted_only_where_it_stands_whole() {
        assert_eq!(redact("hello k1", Some("k1")), "hello [redacted]");
        assert_eq!(redact("Bearer sk-abc", Some("sk-abc")), "Bearer [redacted]");
        assert_eq!(redact("sk-abc,", Some("sk-abc")), "[redacted],");
        assert_eq!(redact("monkey keys 1", Some("key")), "monkey keys 1");
        assert_eq!(redact("as is", None), "as is");
    }

    #[test]
    fn statuses_map_to_what_a_person_can_act_on() {
        let reply = |status: u16, body: &str| {
            ProviderError::from_http_response(
                rig_core::http_client::StatusCode::from_u16(status).unwrap(),
                body,
            )
        };
        assert!(matches!(
            ai_error("p", None, reply(401, "no")),
            AiError::Auth { .. }
        ));
        assert!(matches!(
            ai_error("p", None, reply(403, "no")),
            AiError::Auth { .. }
        ));
        assert!(matches!(
            ai_error("p", None, reply(429, "slow down")),
            AiError::RateLimited { .. }
        ));
        assert_eq!(
            ai_error("p", Some("sk-9"), reply(500, "boom sk-9")),
            AiError::Http {
                provider: "p".into(),
                status: 500,
                body: "boom [redacted]".into()
            }
        );
        assert!(matches!(
            ai_error("p", None, ProviderError::Truncated),
            AiError::BadResponse { .. }
        ));
        assert!(matches!(
            ai_error("p", None, ProviderError::request("no messages")),
            AiError::BadRequest { .. }
        ));
    }
}
