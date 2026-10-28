//! What a declared job runs (`operation: ai.complete_typed`): model rounds
//! streamed on the request's role, written as the attempt's output while
//! they come (`v_job_run_output`), and — for a job that declares `tools` or
//! reads SQL — each tool the model calls run and answered, then another
//! round, up to its `max_tool_rounds`. All of it outside any transaction;
//! cancelling the job drops the model's connection and the attempt closes
//! `cancelled` with what it had. See `.context/extensions.md` "Jobs".

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use oxplow_ai::parts::{
    DeltaFold, MessageRoleV1, MessageV1, PartV1, StreamDeltaV1, ToolDefinitionV1,
};
use oxplow_db::extension_job_store::SqliteExtensionJobStore;
use oxplow_domain::extension_job::{Completion, JobOutputKind, NewJobOutput};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::extension::extension_job_ai::TypedAiJobRequest;
use crate::extension::extension_job_operations::{
    JobOperation, JobOperationError, JobWork, JobWorkOutcome,
};
use crate::extension::extensions::jobs::{JobDecl, QUERY_SQL_TOOL};

/// Tool calls one round may make; more are answered with an error.
const MAX_CALLS_PER_ROUND: usize = 16;
/// Largest tool result handed back to the model, serialized.
const MAX_TOOL_RESULT_BYTES: usize = 65_536;
/// Rows `query_sql` answers with at most.
const QUERY_SQL_ROWS: usize = 200;
/// How often streamed text is written as a row, at most.
const FLUSH_EVERY: Duration = Duration::from_millis(100);
/// Text written as soon as this much has come.
const FLUSH_BYTES: usize = 4096;
/// Longest text one output row holds; longer text is several rows.
const MAX_TEXT_ROW: usize = 65_536;

/// What tool calls run on: the command bus (bound once it exists: the bus
/// is built after the jobs) and the SQL gateway.
pub(crate) struct ToolHost {
    commands: OnceLock<Arc<crate::commands::CommandBus>>,
    sql: crate::sql_gateway::SqlGateway,
}

impl ToolHost {
    pub(crate) fn new(sql: crate::sql_gateway::SqlGateway) -> Self {
        Self {
            commands: OnceLock::new(),
            sql,
        }
    }

    pub(crate) fn bind(&self, commands: Arc<crate::commands::CommandBus>) {
        let _ = self.commands.set(commands);
    }
}

/// One declared job's runner.
pub(crate) struct AiJob {
    pub(crate) ai: Arc<crate::ai_compute::AiCompute>,
    pub(crate) store: SqliteExtensionJobStore,
    pub(crate) tools: Arc<ToolHost>,
    pub(crate) decl: JobDecl,
    /// What the model is told it may call: the declared tools (their
    /// commands' summaries and input schemas) and `query_sql`.
    pub(crate) definitions: Vec<ToolDefinitionV1>,
}

impl AiJob {
    /// The tool definitions `decl` offers, from its extension's `commands`.
    pub(crate) fn definitions(
        decl: &JobDecl,
        commands: &[crate::extension::extension_commands::ExtensionCommand],
    ) -> Vec<ToolDefinitionV1> {
        let mut out: Vec<ToolDefinitionV1> = decl
            .tools
            .iter()
            .filter_map(|tool| {
                let command = commands.iter().find(|c| c.name == tool.command)?;
                Some(ToolDefinitionV1 {
                    name: tool.name.clone(),
                    description: Some(command.summary.clone()),
                    parameters: command.input_schema.clone()?,
                })
            })
            .collect();
        if decl.reads_sql() {
            out.push(ToolDefinitionV1 {
                name: QUERY_SQL_TOOL.into(),
                description: Some(
                    "Run one read-only SQL query (SELECT or WITH) over oxplow's published \
                     datasets, the `v_*` views (`v_dataset` lists them, `v_field` documents \
                     their columns). Positional `?1`, `?2` bind from `params`. At most 200 rows."
                        .into(),
                ),
                parameters: json!({
                    "type": "object",
                    "required": ["sql"],
                    "properties": {
                        "sql": { "type": "string" },
                        "params": { "type": "array" }
                    },
                    "additionalProperties": false
                }),
            });
        }
        out
    }
}

/// How the rounds ended.
enum Ended {
    Done(Completion, Value),
    Stopped(Value),
}

#[async_trait::async_trait]
impl JobOperation for AiJob {
    async fn execute(&self, work: JobWork) -> Result<JobWorkOutcome, JobOperationError> {
        let failed = |error: &str| {
            Ok(JobWorkOutcome::Completed {
                completion: Completion::Failed,
                result: json!({ "error": error }),
            })
        };
        let Ok(input) = TypedAiJobRequest::from_value(work.request()) else {
            return failed("invalid typed AI job input");
        };
        if !self.definitions.is_empty() && !input.tools().is_empty() {
            return failed("this job declares its tools; its request names none");
        }
        let (writer, written) = Writer::spawn(
            self.store.clone(),
            work.job_id().clone(),
            work.attempt_fence(),
        );
        let timeout = Duration::from_secs(u64::from(self.decl.timeout));
        let ended = tokio::time::timeout(timeout, self.rounds(&work, &input, &writer)).await;
        drop(writer);
        // Every row is written before the attempt closes: a closed attempt
        // takes no more.
        let _ = written.await;
        Ok(match ended {
            Ok(Ended::Done(completion, result)) => JobWorkOutcome::Completed { completion, result },
            Ok(Ended::Stopped(result)) => JobWorkOutcome::Stopped {
                result: Some(result),
            },
            Err(_) => JobWorkOutcome::Completed {
                completion: Completion::Failed,
                result: json!({ "error": format!("timed out after {}s", self.decl.timeout) }),
            },
        })
    }
}

impl AiJob {
    async fn rounds(&self, work: &JobWork, input: &TypedAiJobRequest, out: &Writer) -> Ended {
        let fail = |error: String| Ended::Done(Completion::Failed, json!({ "error": error }));
        let model = match self.ai.model_for(input.role()) {
            Ok(model) => model,
            Err(e) => return fail(e.to_string()),
        };
        let mut request = input.request_for_model(&model);
        if !self.definitions.is_empty() {
            request.tools = self.definitions.clone();
        }
        let mut usage = Usage::default();
        let runs_tools = !self.definitions.is_empty();
        for round in 1..=self.decl.max_tool_rounds.max(1) {
            let mut fold = DeltaFold::default();
            let streamed = self
                .ai
                .stream_job_round(work.run(), input.role(), &request, &mut |delta| {
                    fold.push(&delta);
                    out.delta(delta);
                })
                .await;
            if work.run().cancel().is_cancelled() {
                return Ended::Stopped(usage.result(fold.parts(), round, "cancelled"));
            }
            let recorded = match streamed {
                Ok(recorded) => recorded,
                Err(e) => {
                    tracing::warn!(error = %e, "a job's model round failed");
                    return fail(e.to_string());
                }
            };
            usage.add(&recorded);
            out.row(JobOutputKind::Usage, usage.row(&recorded));
            let response = recorded.value;
            let calls: Vec<(String, String, Value)> = response
                .parts
                .iter()
                .filter_map(|part| match part {
                    PartV1::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some((id.clone(), name.clone(), arguments.clone())),
                    _ => None,
                })
                .collect();
            if !runs_tools || calls.is_empty() {
                let mut result = usage.result(&response.parts, round, "");
                result["finishReason"] = json!(response.finish_reason);
                return Ended::Done(Completion::Succeeded, result);
            }
            if round == self.decl.max_tool_rounds {
                let mut result = usage.result(&response.parts, round, "");
                result["error"] = json!("tool rounds exhausted");
                return Ended::Done(Completion::Failed, result);
            }
            request.messages.push(MessageV1 {
                role: MessageRoleV1::Assistant,
                parts: response.parts.clone(),
            });
            let mut results = Vec::new();
            for (index, (id, name, arguments)) in calls.into_iter().enumerate() {
                if work.run().cancel().is_cancelled() {
                    return Ended::Stopped(usage.result(&response.parts, round, "cancelled"));
                }
                let answer = if index >= MAX_CALLS_PER_ROUND {
                    Err(format!("at most {MAX_CALLS_PER_ROUND} tool calls a round"))
                } else {
                    self.call(work, &name, arguments).await
                };
                let (row, result) = match answer {
                    Ok(result) => (json!({ "callId": id, "result": result }), result),
                    Err(error) => (
                        json!({ "callId": id, "error": error }),
                        json!({ "error": error }),
                    ),
                };
                out.row(JobOutputKind::ToolResult, row);
                results.push(PartV1::ToolResult {
                    call_id: id,
                    result,
                });
            }
            request.messages.push(MessageV1 {
                role: MessageRoleV1::Tool,
                parts: results,
            });
        }
        fail("tool rounds exhausted".into())
    }

    /// Run one tool call: `query_sql`, or one of the job's commands under
    /// the job's run (the bus checks it admits a job, validates the input
    /// against its schema, audits it). What it answers, or why not.
    async fn call(&self, work: &JobWork, name: &str, arguments: Value) -> Result<Value, String> {
        if name == QUERY_SQL_TOOL && self.decl.reads_sql() {
            return self.query(arguments).await;
        }
        let tool = self
            .decl
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| format!("no tool `{name}`"))?;
        let commands = self
            .tools
            .commands
            .get()
            .ok_or_else(|| "tools aren't available yet".to_string())?;
        let outcome = commands
            .run_in(work.run(), &tool.command, arguments, false)
            .await
            .map_err(|e| e.to_string())?;
        bounded(if tool.script {
            outcome.result.get("result").cloned().unwrap_or(Value::Null)
        } else {
            outcome.result
        })
    }

    async fn query(&self, arguments: Value) -> Result<Value, String> {
        let sql = arguments
            .get("sql")
            .and_then(Value::as_str)
            .ok_or("`sql` is the query")?;
        let params = match arguments.get("params") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(params)) => params.iter().cloned().map(Into::into).collect(),
            Some(_) => return Err("`params` is a list".into()),
        };
        let out = self
            .tools
            .sql
            .query_sql(sql, params, Some(QUERY_SQL_ROWS))
            .await
            .map_err(|e| e.to_string())?;
        bounded(json!({ "columns": out.columns, "rows": out.rows, "truncated": out.truncated }))
    }
}

/// `value`, unless it is too big to hand back to a model.
fn bounded(value: Value) -> Result<Value, String> {
    let size = serde_json::to_string(&value).map_or(usize::MAX, |s| s.len());
    if size > MAX_TOOL_RESULT_BYTES {
        return Err(format!(
            "its result is {size} bytes; a tool answers at most {MAX_TOOL_RESULT_BYTES}"
        ));
    }
    Ok(value)
}

/// What the attempt's rounds used, summed.
#[derive(Default)]
struct Usage {
    input: i64,
    output: i64,
    calls: Vec<Option<i64>>,
}

impl Usage {
    fn add(&mut self, recorded: &crate::ai_compute::Recorded<oxplow_ai::parts::TypedResponseV1>) {
        self.input += recorded.input_tokens;
        self.output += recorded.output_tokens;
        self.calls.push(recorded.ai_call_id);
    }

    fn row(
        &self,
        recorded: &crate::ai_compute::Recorded<oxplow_ai::parts::TypedResponseV1>,
    ) -> Value {
        json!({
            "inputTokens": recorded.input_tokens,
            "outputTokens": recorded.output_tokens,
            "source": recorded.value.usage.source,
            "aiCallId": recorded.ai_call_id,
            "cached": recorded.cached,
        })
    }

    /// A job's result: the last round's parts, the rounds it took and what
    /// they used; `stopped` names why it ended early, if it did.
    fn result(&self, parts: &[PartV1], rounds: u32, stopped: &str) -> Value {
        let mut result = json!({
            "parts": parts,
            "rounds": rounds,
            "usage": { "inputTokens": self.input, "outputTokens": self.output },
            "aiCalls": self.calls,
        });
        if !stopped.is_empty() {
            result["stopped"] = json!(stopped);
        }
        result
    }
}

/// Writes a running attempt's output: text as it streams, coalesced into a
/// row every [`FLUSH_EVERY`] (or [`FLUSH_BYTES`]), other rows in order
/// with it. Each write is its own short transaction, so nothing is held
/// while the model writes. Once the attempt is closed its rows are refused
/// and the writer stops.
struct Writer {
    tx: mpsc::UnboundedSender<Out>,
}

enum Out {
    Text(String),
    Row(JobOutputKind, Value),
}

impl Writer {
    fn spawn(
        store: SqliteExtensionJobStore,
        job: oxplow_domain::extension_job::JobId,
        token: oxplow_domain::extension_job::AttemptFence,
    ) -> (Writer, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
        let handle = tokio::spawn(async move {
            let mut text = String::new();
            let mut rows: Vec<NewJobOutput> = Vec::new();
            let mut open = true;
            while open {
                // Wait for something, then gather what else comes within
                // the flush window.
                match rx.recv().await {
                    Some(first) => gather(first, &mut text, &mut rows),
                    None => open = false,
                }
                let until = tokio::time::Instant::now() + FLUSH_EVERY;
                while open && text.len() < FLUSH_BYTES {
                    match tokio::time::timeout_at(until, rx.recv()).await {
                        Ok(Some(more)) => gather(more, &mut text, &mut rows),
                        Ok(None) => open = false,
                        Err(_) => break,
                    }
                }
                take_text(&mut text, &mut rows);
                if rows.is_empty() {
                    continue;
                }
                if let Err(e) = store
                    .append_output(job.clone(), token, std::mem::take(&mut rows))
                    .await
                {
                    tracing::debug!(error = %e, "a job's output wasn't written; its attempt ended");
                    return;
                }
            }
        });
        (Writer { tx }, handle)
    }

    fn delta(&self, delta: StreamDeltaV1) {
        let _ = self.tx.send(match delta {
            StreamDeltaV1::Text { text } => Out::Text(text),
            StreamDeltaV1::ToolCall {
                id,
                name,
                arguments,
            } => Out::Row(
                JobOutputKind::ToolCall,
                json!({ "id": id, "name": name, "arguments": arguments }),
            ),
        });
    }

    fn row(&self, kind: JobOutputKind, content: Value) {
        let _ = self.tx.send(Out::Row(kind, content));
    }
}

fn gather(out: Out, text: &mut String, rows: &mut Vec<NewJobOutput>) {
    match out {
        Out::Text(more) => text.push_str(&more),
        Out::Row(kind, content) => {
            take_text(text, rows);
            rows.push(NewJobOutput {
                kind,
                at_ms: oxplow_domain::Timestamp::now().unix_ms(),
                content,
            });
        }
    }
}

/// The text gathered so far as rows, none longer than [`MAX_TEXT_ROW`]
/// (cut on a character boundary).
fn take_text(text: &mut String, rows: &mut Vec<NewJobOutput>) {
    let mut rest = std::mem::take(text);
    while !rest.is_empty() {
        let mut cut = rest.len().min(MAX_TEXT_ROW);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let tail = rest.split_off(cut);
        rows.push(NewJobOutput {
            kind: JobOutputKind::Text,
            at_ms: oxplow_domain::Timestamp::now().unix_ms(),
            content: json!({ "text": rest }),
        });
        rest = tail;
    }
}
