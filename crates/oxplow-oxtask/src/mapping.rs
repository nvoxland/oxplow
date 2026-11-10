//! How oxplow's task fields map onto the task interface: its
//! statuses are its declared stages (one each), its priority the one
//! native field it takes.

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{OxtaskPriority, OxtaskStatus};

/// A status as its stage id (`in_progress`).
pub fn status_str(status: OxtaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("a status serializes as a string")
}

/// The status a stage id names, else `Invalid` at `field`. Core resolved a
/// bucket to its stage before the list sees it, so this is only ever one
/// of oxplow's own stages.
pub fn status_of(stage: &str, field: &str) -> Result<OxtaskStatus, CommandError> {
    serde_json::from_value(Value::String(stage.into())).map_err(|_| CommandError::Invalid {
        field: Some(field.into()),
        message: format!(
            "`{stage}` isn't one of oxplow's stages (backlog, ready, in_progress, blocked, done, \
             canceled)"
        ),
    })
}

/// oxplow's own fields, under `native`: a task's priority.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OxtaskNative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<OxtaskPriority>,
}

/// The `native` an input carries, as oxplow's fields.
pub fn oxtask_native(native: Option<&Value>) -> Result<OxtaskNative, CommandError> {
    match native {
        None => Ok(OxtaskNative::default()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| CommandError::Invalid {
            field: Some("/native".into()),
            message: format!("oxplow's native field is `priority`: {e}"),
        }),
    }
}
