//! What every command handler reaches for, once: its input parsed, a
//! refusal placed at a field, the schema of the type it reads, and a
//! storage error mapped so a busy database retries the run
//! (`.context/commands.md` "`Tx` and `External`") rather than failing it.

use std::sync::{Arc, OnceLock, Weak};

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

/// The bare id a field named for one `resource` carries, parsed (`thr3` →
/// `ThreadId(3)`, a claim's `7` → `7`): the id type's own spelling, never a
/// URI (`.context/resources.md` "Bare ids"). Anything else is refused at
/// `field`, saying the form.
pub fn bare_id<T: std::str::FromStr>(
    raw: &str,
    resource: &str,
    field: &str,
) -> Result<T, CommandError> {
    if let Ok(id) = raw.parse() {
        return Ok(id);
    }
    let hint = match oxplow_domain::uri::build::id_of(raw, resource) {
        Ok(id) => format!("; pass the id (`{id}`), not its URI"),
        Err(_) => String::new(),
    };
    Err(invalid(
        field,
        format!("`{raw}` isn't an {resource} id{hint}"),
    ))
}

/// The services the operations that reach across them run against, filled
/// once there is an `Arc` (`Services::fill_slot`): the operations are
/// added while services are being built, before there is one to point at.
#[derive(Clone, Default)]
pub struct ServicesSlot(Arc<OnceLock<Weak<crate::Services>>>);

impl ServicesSlot {
    pub fn fill(&self, services: &Arc<crate::Services>) {
        let _ = self.0.set(Arc::downgrade(services));
    }

    /// The services, or a refusal when they are gone (shutting down).
    pub fn get(&self) -> Result<Arc<crate::Services>, CommandError> {
        self.0
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| CommandError::Failed {
                message: "oxplow is shutting down".into(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, ThreadId};

    /// A field named for one resource carries its bare id, and only it: a
    /// URI, another id type's spelling or an empty one is refused at the
    /// field, saying the form.
    #[test]
    fn a_fixed_field_takes_a_bare_id() {
        assert_eq!(
            bare_id::<ThreadId>("thr3", oxplow_domain::uri::resources::THREAD, "/thread").unwrap(),
            ThreadId::new(3)
        );
        assert_eq!(
            bare_id::<StreamId>("str1", oxplow_domain::uri::resources::STREAM, "/stream").unwrap(),
            StreamId::new(1)
        );
        assert_eq!(
            bare_id::<i64>("7", oxplow_domain::uri::resources::CLAIM, "/claim").unwrap(),
            7
        );
        for raw in ["str1", "", "3", "thread:thr3", "nope"] {
            let err = bare_id::<ThreadId>(raw, oxplow_domain::uri::resources::THREAD, "/thread")
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message }
                    if f == "/thread" && message.contains("isn't an oxplow/thread id")),
                "{raw}: {err:?}"
            );
        }
        let err = bare_id::<ThreadId>(
            "oxplow:oxplow/thread?id=thr3",
            oxplow_domain::uri::resources::THREAD,
            "/thread",
        )
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. }
                if message.contains("pass the id (`thr3`), not its URI")),
            "{err:?}"
        );
    }
}
