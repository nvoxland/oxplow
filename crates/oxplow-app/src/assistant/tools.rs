//! The Assistant's tools: bus commands the person may run
//! (`.context/ai-providers.md` "The Assistant"). Each read (`View`,
//! `Read`) is a Rig `DynamicTool` of its own, always offered. Changes
//! (`Record`, `Write`) are offered only while "Allow changes" is on, and
//! through two tools rather than one each — a model is offered at most
//! 128 tools by some wires, and every change's schema in every request
//! would be most of it: [`FIND_COMMAND`] lists the changes with their
//! input schemas, [`RUN_COMMAND`] runs one, after the person allows it
//! on its card (the session's hook). A run is the person's
//! (`Actor::Human`), acting through the Assistant session, so its audit
//! row says `via: assistant:<id>`.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_ai_rig::rig_core::message::ToolName;
use oxplow_ai_rig::rig_core::tool::{
    ContextValue, DynamicTool, ToolContext, ToolErrorKind, ToolExecutionError, ToolOutput,
};
use oxplow_domain::run::{Mode, RunContext};
use oxplow_domain::{Actor, CommandSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::CommandBus;

/// Largest result handed back to the model, serialized (as a job's tool).
pub const MAX_TOOL_RESULT_BYTES: usize = 65_536;

/// The tool that lists the changes the model may ask to run.
pub const FIND_COMMAND: &str = "oxplow_find_command";
/// The tool that runs a change, once the person allows it.
pub const RUN_COMMAND: &str = "oxplow_run_command";

/// Changes [`FIND_COMMAND`] lists at most.
const FOUND_AT_MOST: usize = 20;

/// What a tool name stands for.
#[derive(Debug, Clone)]
pub enum ToolSpec {
    /// A read: the bus command it runs, and its summary.
    Read { command: String, summary: String },
    /// [`FIND_COMMAND`]: a read of the bus's catalog.
    Find,
    /// [`RUN_COMMAND`]: a change, named by its call's `command`.
    Run,
}

/// The tools of one turn: Rig's, and what each name stands for.
pub struct Tools {
    pub tools: Vec<DynamicTool>,
    pub specs: Arc<BTreeMap<String, ToolSpec>>,
}

/// The audit row a tool's run made, handed from the executor to the
/// session's hook on the tool's result context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AuditRef(pub i64);

impl ContextValue for AuditRef {
    const KEY: &'static str = "oxplow.audit";
}

/// A command id as a tool name: `.` → `_` (`oxplow.vcs.log` →
/// `oxplow_vcs_log`), as a job's tools are named.
pub fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

/// Whether `spec` is offered as a tool of its own: a read with an object
/// input schema (a model can't be told about any other).
fn read_tool(spec: &CommandSpec) -> bool {
    spec.access.reads_only() && spec.input_schema.is_object()
}

/// A change the person may run (`Record`, `Write`), for [`RUN_COMMAND`]:
/// its spec, or why not.
pub fn changing_command(bus: &CommandBus, command: &str) -> Result<CommandSpec, String> {
    let spec = bus
        .list(&Actor::Human)
        .into_iter()
        .find(|s| s.id == command)
        .ok_or_else(|| format!("there is no command `{command}`; `{FIND_COMMAND}` lists them"))?;
    if spec.access.reads_only() {
        return Err(format!(
            "`{command}` only reads: call its own tool, `{}`",
            tool_name(command)
        ));
    }
    Ok(spec)
}

/// [`FIND_COMMAND`]: the changes whose id or summary holds every word of
/// `query` (all of them with none), with their input schemas.
pub fn find(bus: &CommandBus, args: &Value) -> Value {
    let words: Vec<String> = args["query"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let mut found: Vec<CommandSpec> = bus
        .list(&Actor::Human)
        .into_iter()
        .filter(|s| s.access.records())
        .filter(|s| {
            let text = format!("{} {}", s.id, s.summary).to_lowercase();
            words.iter().all(|w| text.contains(w))
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    let more = found.len() > FOUND_AT_MOST;
    let commands: Vec<Value> = found
        .into_iter()
        .take(FOUND_AT_MOST)
        .map(|s| json!({ "command": s.id, "summary": s.summary, "input_schema": s.input_schema }))
        .collect();
    json!({ "commands": commands, "more": more })
}

/// A command's input schema as a tool's parameters: an object schema
/// (some wires refuse anything else).
fn parameters(schema: &Value) -> Value {
    let mut schema = schema.clone();
    if let Some(object) = schema.as_object_mut() {
        object.entry("type").or_insert_with(|| json!("object"));
        object.entry("properties").or_insert_with(|| json!({}));
    }
    schema
}

/// The tools a turn offers, from the bus's commands the person may run
/// (`CommandBus::list(&Actor::Human)`), sorted by name; with
/// `allow_changes`, [`FIND_COMMAND`] and [`RUN_COMMAND`] too. `sql` false
/// (the binding forbids the `query_sql` tool) leaves out
/// `oxplow.sql.query`.
pub fn tools(bus: &Arc<CommandBus>, via: &str, allow_changes: bool, sql: bool) -> Tools {
    let mut specs: Vec<CommandSpec> = bus
        .list(&Actor::Human)
        .into_iter()
        .filter(read_tool)
        .filter(|s| sql || s.id != crate::commands::agent_reads::sql::SQL_QUERY)
        .collect();
    specs.sort_by(|a, b| a.id.cmp(&b.id));
    let mut named: BTreeMap<String, ToolSpec> = BTreeMap::new();
    let mut out = Vec::new();
    let runner = |command: Option<String>| Runner {
        bus: bus.clone(),
        command,
        via: via.to_string(),
    };
    for spec in specs {
        let name = tool_name(&spec.id);
        // Two ids that differ only by `.` and `_` would share a name: the
        // first keeps it.
        if named.contains_key(&name) {
            continue;
        }
        let Some(tool) = dynamic(
            &name,
            &spec.summary,
            parameters(&spec.input_schema),
            runner(Some(spec.id.clone())),
        ) else {
            continue;
        };
        out.push(tool);
        named.insert(
            name,
            ToolSpec::Read {
                command: spec.id,
                summary: spec.summary,
            },
        );
    }
    if allow_changes {
        let find_tool = {
            let bus = bus.clone();
            ToolName::new(FIND_COMMAND).ok().map(|name| {
                DynamicTool::new(
                    name,
                    "List the commands that change something (files, git, tasks, settings), \
                     with their input schemas: every one whose id or summary holds each word \
                     of `query`. Run one with `oxplow_run_command`.",
                    json!({
                        "type": "object",
                        "properties": { "query": { "type": "string" } },
                        "additionalProperties": false
                    }),
                    move |args| {
                        let found = find(&bus, &args);
                        Box::pin(async move { bounded(found).map(ToolOutput::json) })
                    },
                )
            })
        };
        let run_tool = dynamic(
            RUN_COMMAND,
            "Run a command that changes something: `command` is its id (from \
             `oxplow_find_command`), `input` its input. The person is asked first.",
            json!({
                "type": "object",
                "required": ["command", "input"],
                "properties": {
                    "command": { "type": "string" },
                    "input": { "type": "object" }
                },
                "additionalProperties": false
            }),
            runner(None),
        );
        for (name, tool, spec) in [
            (FIND_COMMAND, find_tool, ToolSpec::Find),
            (RUN_COMMAND, run_tool, ToolSpec::Run),
        ] {
            if let Some(tool) = tool {
                out.push(tool);
                named.insert(name.to_string(), spec);
            }
        }
    }
    Tools {
        tools: out,
        specs: Arc::new(named),
    }
}

/// A tool over `run`; `None` for a name a wire can't carry.
fn dynamic(name: &str, description: &str, parameters: Value, run: Runner) -> Option<DynamicTool> {
    let name = ToolName::new(name).ok()?;
    Some(DynamicTool::new_with_context(
        name,
        description.to_string(),
        parameters,
        move |context: &mut ToolContext, args| {
            let run = run.clone();
            Box::pin(async move { run.call(context, args).await })
        },
    ))
}

/// The one executor: run a command as the person, through the session —
/// its own (a read), or the call's `command` (a change, [`RUN_COMMAND`]).
#[derive(Clone)]
struct Runner {
    bus: Arc<CommandBus>,
    command: Option<String>,
    via: String,
}

impl Runner {
    async fn call(
        &self,
        context: &mut ToolContext,
        args: Value,
    ) -> Result<ToolOutput, ToolExecutionError> {
        let failed = |message: String| ToolExecutionError::new(ToolErrorKind::Other, message);
        let (command, input, changes) = match &self.command {
            Some(command) => (command.clone(), args, false),
            None => {
                let command = args["command"].as_str().unwrap_or_default().to_string();
                changing_command(&self.bus, &command).map_err(failed)?;
                (command, args["input"].clone(), true)
            }
        };
        let run = RunContext::core(Actor::Human, Mode::External).acting_through(&self.via);
        // A change ran only once the person allowed it on its card: that
        // is their confirmation of this exact call.
        let outcome = self
            .bus
            .run_in(&run, &command, input, changes)
            .await
            .map_err(|e| failed(e.to_string()))?;
        if let Some(audit) = outcome.audit_id {
            let _ = context.insert_result(AuditRef(audit));
        }
        bounded(outcome.result).map(ToolOutput::json)
    }
}

/// `value`, unless it is too big to hand back to a model.
fn bounded(value: Value) -> Result<Value, ToolExecutionError> {
    let size = serde_json::to_string(&value).map_or(usize::MAX, |s| s.len());
    if size > MAX_TOOL_RESULT_BYTES {
        return Err(ToolExecutionError::new(
            ToolErrorKind::Other,
            format!("its result is {size} bytes; a tool answers at most {MAX_TOOL_RESULT_BYTES}"),
        ));
    }
    Ok(value)
}
