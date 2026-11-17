//! The levels of the shared settings service.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One settings level: which override file (or its secure-store entry) a
/// value lives in. The one level type for storage, commands and events.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// This person's override for this project (`.oxplow/local.yaml`).
    Local,
    /// The project's shared value (`.oxplow/project.yaml`).
    Project,
    /// The person's default across projects (`global.yaml`).
    Global,
}

impl Level {
    /// The levels in the order a read looks: the first that sets a key wins.
    pub const PRECEDENCE: [Self; 3] = [Self::Local, Self::Project, Self::Global];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}
