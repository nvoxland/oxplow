//! Validated `jobs:` declarations. A declaration names a host operation;
//! scheduling and execution are separate host services.

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

const MAX_JOBS: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 4096;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobReplay {
    #[default]
    Never,
    Idempotent,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
pub enum JobOperationKind {
    #[serde(rename = "ai.complete_typed")]
    AiCompleteTyped,
}

impl JobOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AiCompleteTyped => "ai.complete_typed",
        }
    }
}

/// A host-recognized operation owned by one extension contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobDecl {
    pub id: String,
    pub description: String,
    pub version: u32,
    pub operation: JobOperationKind,
    pub needs: Vec<String>,
    pub replay: JobReplay,
}

/// The on-disk shape, with unknown fields refused by serde.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobFile {
    id: String,
    description: String,
    version: u32,
    operation: JobOperationKind,
    needs: Vec<String>,
    #[serde(default)]
    replay: JobReplay,
}

pub fn parse_jobs(value: &Value, file: &str, manifest: &str) -> (Vec<JobDecl>, Vec<String>) {
    let block = key_line(manifest, "jobs");
    let Some(items) = value.as_sequence() else {
        return (Vec::new(), vec![at(file, block, "`jobs` must be a list")]);
    };
    if items.len() > MAX_JOBS {
        return (
            Vec::new(),
            vec![at(file, block, "`jobs` may declare at most 128 operations")],
        );
    }

    let lines = item_lines(manifest, "jobs");
    let mut jobs: Vec<JobDecl> = Vec::with_capacity(items.len());
    let mut errors = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let line = lines.get(index).copied().or(block);
        let Ok(raw) = serde_yaml::from_value::<JobFile>(item.clone()) else {
            errors.push(at(file, line, "job declaration has invalid fields"));
            continue;
        };
        if raw.id.len() > 256
            || !oxplow_domain::contribution::valid_local(
                oxplow_domain::contribution::ContributionKind::Job,
                &raw.id,
            )
        {
            errors.push(at(file, line, "job id has invalid syntax"));
        } else if raw.description.len() > MAX_DESCRIPTION_BYTES {
            errors.push(at(file, line, "job description exceeds 4096 bytes"));
        } else if raw.version == 0 {
            errors.push(at(file, line, "job version must be positive"));
        } else if raw.needs.len() != 1 || raw.needs[0] != "ai.compute" {
            errors.push(at(file, line, "job needs must be exactly [ai.compute]"));
        } else if jobs.iter().any(|job| job.id == raw.id) {
            errors.push(at(file, line, "job id is declared twice"));
        } else {
            jobs.push(JobDecl {
                id: raw.id,
                description: raw.description,
                version: raw.version,
                operation: raw.operation,
                needs: raw.needs,
                replay: raw.replay,
            });
        }
    }
    (jobs, errors)
}
