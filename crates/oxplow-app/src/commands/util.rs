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
