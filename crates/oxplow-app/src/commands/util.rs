//! What every command handler reaches for, once: its input parsed, a
//! refusal placed at a field, the schema of the type it reads, and a
//! storage error mapped so a busy database retries the run
//! (`.context/commands.md` "`Tx` and `External`") rather than failing it.

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// `input` as the type a handler reads. The bus has checked it against
/// the schema already, so a failure here is the schema and the type
/// disagreeing.
pub fn parse<T: DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

/// A refusal of the input at `field` (a JSON pointer: `/thread`).
pub fn invalid(field: &str, message: impl Into<String>) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message: message.into(),
    }
}

/// The JSON Schema of `T`: an operation's input schema.
pub fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("a schema serializes")
}

/// A SQLite error as a command's: `Busy` (retried) when the database was
/// locked, else a failure.
pub fn sql(e: rusqlite::Error) -> CommandError {
    CommandError::from(oxplow_db::map_sql_err(e))
}

/// A failure, saying why.
pub fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed {
        message: e.to_string(),
    }
}

/// The id a `<kind>:<id>` ref names (`thread:thr3` → `thr3`) — the one way
/// a command's input names a record (`.context/refs.md`); anything else is
/// refused at `field`, saying the form.
pub fn ref_body<'a>(raw: &'a str, kind: &str, field: &str) -> Result<&'a str, CommandError> {
    raw.strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix(':'))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid(field, format!("`{raw}` isn't a {kind} ref ({kind}:<id>)")))
}

/// [`ref_body`], parsed: `thread:thr3` → `ThreadId(3)`.
pub fn ref_id<T: std::str::FromStr>(raw: &str, kind: &str, field: &str) -> Result<T, CommandError> {
    ref_body(raw, kind, field)?
        .parse()
        .map_err(|_| invalid(field, format!("`{raw}` isn't a {kind} ref ({kind}:<id>)")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, ThreadId};

    /// A record is named by its ref, and only by it: a bare id, another
    /// kind's ref or an empty one is refused at the field, saying the form.
    #[test]
    fn a_record_is_named_by_its_ref() {
        assert_eq!(
            ref_id::<ThreadId>("thread:thr3", "thread", "/thread").unwrap(),
            ThreadId::new(3)
        );
        assert_eq!(
            ref_id::<StreamId>("stream:str1", "stream", "/stream").unwrap(),
            StreamId::new(1)
        );
        for raw in [
            "thr3",
            "stream:str1",
            "thread:",
            "threads:thr3",
            "thread:nope",
        ] {
            let err = ref_id::<ThreadId>(raw, "thread", "/thread").unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message }
                    if f == "/thread" && message.contains("isn't a thread ref (thread:<id>)")),
                "{raw}: {err:?}"
            );
        }
    }
}
