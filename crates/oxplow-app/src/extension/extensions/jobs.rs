//! Validated `jobs:` declarations. A declaration names a host operation;
//! scheduling and execution are separate host services.

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

const MAX_JOBS: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 4096;

/// Model rounds a job with tools runs at most, by default and at most.
pub const DEFAULT_TOOL_ROUNDS: u32 = 4;
pub const MAX_TOOL_ROUNDS: u32 = 16;
/// How long a job may run, in seconds, by default and at most.
pub const DEFAULT_TIMEOUT_SECS: u32 = 120;
pub const MAX_TIMEOUT_SECS: u32 = 600;

/// The tool a job with `sql.read` offers its model besides its own: a
/// read-only query over the published datasets, as MCP `oxplow_query_sql` is.
pub const QUERY_SQL_TOOL: &str = "query_sql";

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
    /// An attempt cut off (a restart, a lost lease) ends the job
    /// `interrupted`.
    #[default]
    Never,
    /// The job can run again from its first request: an attempt cut off
    /// is queued again, up to `MAX_ATTEMPTS` in all, and its
    /// `contribution.job.completed` is logged once, at its final end.
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
    /// The extension's commands its model may call, as the model sees them.
    pub tools: Vec<JobTool>,
    pub max_tool_rounds: u32,
    /// Seconds it may run before it fails.
    pub timeout: u32,
}

/// One of a job's tools: the command it runs and the name the model calls
/// it by (its local name, `.` as `_`: model APIs allow no dots).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobTool {
    pub name: String,
    pub command: String,
    /// A script command: what it answers is its script's `result` (the
    /// commands it composed are its own business).
    pub script: bool,
}

impl JobDecl {
    /// Whether its dataset may read the published datasets (`query_sql`).
    pub fn reads_sql(&self) -> bool {
        self.needs.iter().any(|need| need == "sql.read")
    }
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
    /// This extension's commands (their names as declared, without the
    /// namespace) the model may call; each says `job: true`.
    #[serde(default)]
    tools: Vec<String>,
    /// Model rounds at most (1–16, default 4): a round that calls tools
    /// is followed by another with their results.
    #[serde(default)]
    max_tool_rounds: Option<u32>,
    /// Seconds it may run (1–600, default 120) before it fails.
    #[serde(default)]
    timeout: Option<u32>,
}

/// `jobs:`, checked against the extension's `commands` (in `namespace`):
/// a tool is one of them that admits a job and says its input.
pub fn parse_jobs(
    value: &Value,
    file: &str,
    manifest: &str,
    namespace: &str,
    commands: &[crate::extension::extension_commands::ExtensionCommand],
) -> (Vec<JobDecl>, Vec<String>) {
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
        } else if !needs_hold(&raw.needs) {
            errors.push(at(
                file,
                line,
                "job needs are [ai.compute], or [ai.compute, sql.read] to let its model query",
            ));
        } else if jobs.iter().any(|job| job.id == raw.id) {
            errors.push(at(file, line, "job id is declared twice"));
        } else if raw
            .max_tool_rounds
            .is_some_and(|n| !(1..=MAX_TOOL_ROUNDS).contains(&n))
        {
            errors.push(at(file, line, "job max_tool_rounds is 1 to 16"));
        } else if raw
            .timeout
            .is_some_and(|n| !(1..=MAX_TIMEOUT_SECS).contains(&n))
        {
            errors.push(at(file, line, "job timeout is 1 to 600 seconds"));
        } else {
            match tools(&raw, namespace, commands) {
                Ok(tools) => jobs.push(JobDecl {
                    id: raw.id,
                    description: raw.description,
                    version: raw.version,
                    operation: raw.operation,
                    needs: raw.needs,
                    replay: raw.replay,
                    tools,
                    max_tool_rounds: raw.max_tool_rounds.unwrap_or(DEFAULT_TOOL_ROUNDS),
                    timeout: raw.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS),
                }),
                Err(e) => errors.push(at(file, line, &e)),
            }
        }
    }
    (jobs, errors)
}

/// `[ai.compute]`, or with `sql.read` besides.
pub(crate) fn needs_hold(needs: &[String]) -> bool {
    let mut sorted: Vec<&str> = needs.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted == ["ai.compute"] || sorted == ["ai.compute", "sql.read"]
}

/// A job's `tools`, each one of `commands` that admits a job and says its
/// input; their model-facing names distinct, and none `query_sql`.
fn tools(
    raw: &JobFile,
    namespace: &str,
    commands: &[crate::extension::extension_commands::ExtensionCommand],
) -> Result<Vec<JobTool>, String> {
    let mut out: Vec<JobTool> = Vec::new();
    for local in &raw.tools {
        let command = format!("{namespace}.{local}");
        let Some(declared) = commands.iter().find(|c| c.name == command) else {
            return Err(format!(
                "job tool `{local}` is none of this extension's commands"
            ));
        };
        if !declared.job {
            return Err(format!(
                "job tool `{local}` doesn't admit a job: its command says `job: true`"
            ));
        }
        if !declared.access.reads_only() {
            return Err(format!(
                "job tool `{local}` writes: a tool only reads (`access: read`); write from an \
                 effect on the job's `contribution.job.completed`"
            ));
        }
        if declared.input_schema.is_none() {
            return Err(format!(
                "job tool `{local}` has no `input_schema`: the model is told what it takes"
            ));
        }
        let name = local.replace('.', "_");
        if name == QUERY_SQL_TOOL || out.iter().any(|t| t.name == name) {
            return Err(format!("job tool `{local}` is named like another tool"));
        }
        out.push(JobTool {
            name,
            command,
            script: matches!(
                declared.handler,
                crate::extension::extension_commands::CommandHandler::Script { .. }
            ),
        });
    }
    Ok(out)
}
