//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Tools are thin handlers that
//! delegate into `oxplow-app` services — we never duplicate business
//! logic between the Tauri command surface and the MCP tool surface.
//!
//! Each tool takes a single `Parameters<T>` argument (rmcp
//! convention); request shapes are defined as `serde + JsonSchema`
//! structs alongside the tool methods.

use oxplow_app::commands::agent_reads::ai::{AiDecideParams, AiSummarizeParams};
use oxplow_app::commands::agent_reads::changes::EnsureChangeParams;
use oxplow_app::commands::agent_reads::collectors::PreviewCollectorParams;
use oxplow_app::commands::agent_reads::extensions::{
    CheckError, ExtensionChecks, ReviewExtensionParams, ValidateExtensionParams,
};
use oxplow_app::commands::agent_reads::knowledge::{ListCommentsParams, WikiRefDriftParams};
use oxplow_app::commands::agent_reads::lsp::{
    CodeCallHierarchyParams, CodePositionParams, CodeReferencesParams, CodeSymbolsParams,
    CodeWorkspaceSymbolsParams,
};
use oxplow_app::commands::agent_reads::vcs::{BlameParams, DiffParams};
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::*;
use rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::session_auth::Principal;
use oxplow_app::Services;

mod lenient_params;
// Drop-in for rmcp's `Parameters` that tolerates camelCase/kebab aliases
// of our snake_case param fields, so weak models stop tripping on
// `-32602 missing field`. See `lenient_params` for the rationale.
use lenient_params::Parameters;
use oxplow_app::commands::agent_reads::lens::{RunLensActionInput, RunLensInput};
use oxplow_app::commands::agent_reads::pages::OpenPageInput;
use oxplow_app::commands::agent_reads::records::ReadInput;
use oxplow_app::commands::agent_reads::search::SiteSearchInput;
use oxplow_app::commands::agent_reads::sql::QuerySqlInput;

#[derive(Clone)]
pub struct OxplowMcp {
    services: Arc<Services>,
    tool_router: ToolRouter<Self>,
}

/// The extension checks `oxplow.extension.validate` and `.preview` run
/// (`oxplow-sdk`'s, which `oxplow-app` can't link): installed on the
/// services of every host that serves these tools. Idempotent.
pub fn install_extension_checks(services: &Services) {
    services.extension_checks.install(Arc::new(SdkChecks));
}

struct SdkChecks;

#[async_trait::async_trait]
impl ExtensionChecks for SdkChecks {
    async fn validate(
        &self,
        svc: &Services,
        root: &std::path::Path,
        name: &str,
    ) -> Result<serde_json::Value, CheckError> {
        let report = oxplow_sdk::check(
            root,
            name,
            &svc.extension_catalog,
            Some(&svc.sql),
            Some(svc.commands.as_ref()),
            None,
        )
        .await
        .map_err(sdk_check_error)?;
        serde_json::to_value(report).map_err(|e| CheckError::Failed(e.to_string()))
    }

    async fn preview(
        &self,
        root: &std::path::Path,
        name: &str,
    ) -> Result<serde_json::Value, CheckError> {
        let report = oxplow_sdk::extension_test::test_extension_in(
            root,
            name,
            false,
            Arc::new(|_: &str| None),
        )
        .await
        .map_err(sdk_check_error)?;
        serde_json::to_value(report).map_err(|e| CheckError::Failed(e.to_string()))
    }
}

fn sdk_check_error(e: oxplow_sdk::SdkError) -> CheckError {
    match e {
        oxplow_sdk::SdkError::NotFound(_) => CheckError::NotFound,
        other => CheckError::Failed(other.to_string()),
    }
}

// ---------- caller identity ----------

/// Who is calling: the agent session, thread and stream behind an MCP
/// request.
///
/// The control plane admits an MCP request only with an agent session's
/// bearer and puts that session's [`Principal`] in the request's
/// extensions (`.context/agent-model.md` "Caller identity"); rmcp hands the
/// request's `http::request::Parts` to tools through the call's
/// `Extensions`, and [`caller_of`] reads the principal there. Nothing the
/// request says about itself — a header, the URL — names the caller. A
/// transport with no HTTP parts (stdio, a unit test) is an anonymous
/// agent: it may read, and `run_command` refuses to write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpCaller {
    pub session_id: Option<oxplow_domain::AgentSessionId>,
    pub thread_id: Option<oxplow_domain::ThreadId>,
    pub stream_id: Option<oxplow_domain::StreamId>,
}

impl McpCaller {
    pub fn from_parts(parts: &http::request::Parts) -> Self {
        parts
            .extensions
            .get::<Principal>()
            .map(Self::of)
            .unwrap_or_default()
    }

    /// The caller a session's bearer stands for.
    pub fn of(principal: &Principal) -> Self {
        Self {
            session_id: Some(principal.session),
            thread_id: Some(principal.thread),
            stream_id: Some(principal.stream),
        }
    }

    pub fn actor(&self) -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
            session_id: self.session_id,
            thread_id: self.thread_id,
            stream_id: self.stream_id,
        }
    }
}

/// The caller behind a tool call, from the request extensions rmcp
/// attaches; anonymous when the transport carried no HTTP parts.
pub fn caller_of(extensions: &rmcp::model::Extensions) -> McpCaller {
    extensions
        .get::<http::request::Parts>()
        .map(McpCaller::from_parts)
        .unwrap_or_default()
}

/// What `run_command` says to a caller with no thread identity.
const ANONYMOUS_WRITE: &str = "this MCP connection carries no thread identity (it didn't come \
    through an agent session's bearer), so it may not run commands; oxplow's own harness \
    configs connect with one — reconnect through one";

// ---------- request shapes ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunCommandParams {
    /// The command id, as `list_commands` reports it (`oxplow.config.set`,
    /// `oxplow.work_item.transition`).
    pub id: String,
    /// The command's input, matching its `input_schema`.
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListCommandsParams {
    /// One command's id (`oxplow.work_item.create`): it alone, in full.
    #[serde(default)]
    pub id: Option<String>,
    /// The start of the ids wanted (`oxplow.work_item.`): each, in full.
    #[serde(default)]
    pub prefix: Option<String>,
}

/// A summary's first sentence: what the index shows (the whole summary
/// comes with the command).
fn first_sentence(summary: &str) -> &str {
    // A sentence ends at ". " before a capital: `e.g. no` goes on.
    summary
        .match_indices(". ")
        .find(|(at, _)| {
            summary[at + 2..]
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
        })
        .map_or(summary, |(at, _)| &summary[..=at])
}

/// A command as `list_commands`' index shows it.
#[derive(Debug, Serialize)]
struct CommandIndexEntry {
    id: String,
    summary: String,
}

/// A command as an agent needs it to run one: the person-facing parts of
/// its spec (`ui`, `op`, `atomicity`, …) left out, and its example input
/// shown once, beside the schema rather than inside it.
#[derive(Debug, Serialize)]
struct CommandView {
    id: String,
    summary: String,
    input_schema: serde_json::Value,
    example: Option<serde_json::Value>,
    confirm: oxplow_domain::Confirm,
    undoable: bool,
}

impl CommandView {
    fn of(spec: &oxplow_domain::CommandSpec) -> Self {
        let mut input_schema = spec.input_schema.clone();
        let example = input_schema
            .as_object_mut()
            .and_then(|schema| schema.remove("examples"))
            .and_then(|examples| examples.as_array()?.first().cloned());
        Self {
            id: spec.id.clone(),
            summary: spec.summary.clone(),
            input_schema,
            example,
            confirm: spec.confirm,
            undoable: spec.undoable,
        }
    }
}

/// Why `list_commands` found nothing: the id or prefix isn't offered, and
/// the offered commands whose ids end like the asked one.
fn no_command_message(
    offered: &[oxplow_domain::CommandSpec],
    id: Option<&str>,
    prefix: Option<&str>,
) -> String {
    let asked = id.or(prefix).unwrap_or_default();
    let ending_in = |suffix: &str| -> Vec<&str> {
        offered
            .iter()
            .map(|spec| spec.id.as_str())
            .filter(|candidate| !suffix.is_empty() && candidate.ends_with(&format!(".{suffix}")))
            .collect()
    };
    // `config.get` → `oxplow.config.get`; else by its last segment.
    let whole = asked.trim_matches('.');
    let mut alike = ending_in(whole);
    if alike.is_empty() {
        alike = ending_in(whole.rsplit('.').next().unwrap_or(whole));
    }
    let what = if id.is_some() {
        "command"
    } else {
        "commands under"
    };
    if alike.is_empty() {
        format!(
            "no {what} `{asked}` is offered to you; `list_commands` with no arguments lists them"
        )
    } else {
        format!(
            "no {what} `{asked}` is offered to you; did you mean {}?",
            alike
                .iter()
                .map(|candidate| format!("`{candidate}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkItemRefParams {
    /// The item's ref, `work_item:<provider>:<id>`.
    #[serde(rename = "ref")]
    pub item_ref: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReadAtParams {
    /// The stream whose workspace to read; the caller's when absent.
    pub stream_id: Option<String>,
    /// Workspace-relative file path.
    pub path: String,
    /// Which version: `working`, `snap:<snapshot id>`, or `git:<rev>`
    /// (a sha, branch, tag or `HEAD`).
    pub revision: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StreamScopeParams {
    /// Stream whose worktree to read `oxplow/extensions/` from. Omit for
    /// your own stream (the calling thread's; the primary when the call
    /// carries no thread).
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ShowLensParams {
    /// An existing lens to show (`<extension>/<slug>`, see `v_lens`).
    /// Give this or `spec`.
    pub lens: Option<String>,
    /// A lens of your own: `title`, `query` (read-only SQL over the `v_*`
    /// models, `:param` bindings), `viz` (`table`, `list`, `number`,
    /// `markdown`, `bar`, `line`, `treemap`, `tree`, `timeline`, `detail`,
    /// `steps`, `hunks`) and what the viz needs (`chart: {x, y, series}`,
    /// `tree: {id, parent, label}`, …; the same keys as a lens file).
    pub spec: Option<oxplow_app::extensions::LensSpec>,
    /// Param values by name.
    pub params: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LensIdParams {
    /// Lens id: `<extension>/<slug>` (see `v_lens`).
    pub id: String,
    /// Stream whose worktree to read from; omit for your own.
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReadAiCallParams {
    /// The call's id (`v_ai_call.id`).
    pub id: i64,
    /// Which body: `request` or `response`.
    pub body: oxplow_app::ai_calls::AiCallBody,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSkillParams {
    /// Skill name, e.g. `oxplow-extension`.
    pub name: String,
    /// One of its reference files, by name (the skill names them as
    /// `references/<name>.md`); absent, the skill's `SKILL.md`.
    #[serde(default)]
    pub reference: Option<String>,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        install_extension_checks(&services);
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Read one of oxplow's skills (its SKILL.md): how to do a kind of oxplow \
                       work, e.g. `oxplow-extension` for building lenses. A skill sends you to \
                       `references/<name>.md` for detail: pass that `<name>` as `reference`. For \
                       agents that don't load skill files themselves; your instructions list the \
                       names."
    )]
    async fn get_skill(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<GetSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let GetSkillParams { name, reference } = params.0;
        self.read_command(
            &caller_of(&extensions),
            "oxplow.skill.read",
            serde_json::json!({ "name": name, "reference": reference }),
        )
        .await
    }

    #[tool(
        description = "What the human is looking at right now in a thread: the open page's id \
                       (`task:42`, `file:src/a.rs`, `lens:review/waiting`, …), its kind and \
                       page detail. When it's a lens, `lens` is that lens re-run with the \
                       human's current params, as text: exactly what is on their screen. Use it when \
                       the user says \"this\", \"what I'm looking at\" or \"this lens\". \
                       `open` is null when nothing has been reported."
    )]
    async fn get_open_page(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<OpenPageInput>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.page.open",
            serde_json::to_value(&params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Dry-run a collector from a stream's worktree and return what it would \
                       store, storing nothing: an entity collector's rows (first 50 per entity, \
                       coerced to the declared columns), or a fact collector's (`facts:`) facts \
                       as `{ owner, id, facts: [{ measure, value, subject, … }] }`, computed over \
                       the stream's latest snapshot (a collector that reads `input` rows is \
                       previewed through an example: `oxplow extension test`). Use it to check a collector you're writing, especially in a \
                       worktree stream: oxplow.collector.sync always runs the primary's copy, and \
                       collected data is project-wide. Pass `event` (`{ type, payload, anchors? }`) \
                       to preview an `on:` collector on the event it reacts to. An exec \
                       collector still needs a person's approval of that exact version; \
                       starlark/jaq collectors run without it."
    )]
    async fn preview_collector(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<PreviewCollectorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.collector.preview",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Analyze a change and return its row (`v_change`): a commit vs its parent, an \
                       effort (start → end), or a stream's uncommitted work vs HEAD. Then read the \
                       analysis with query_sql: v_change_file (files, zones), \
                       v_oxplow_bundled_change_interest (look-here-first scores, when \
                       oxplow-bundled is on), v_change_function (added/deleted/modified functions, deltas, \
                       churn), v_change_import (cross-zone imports), \
                       v_oxplow_bundled_change_co_change (files whose usual partners are missing, \
                       or that were dormant; oxplow-bundled), v_change_duplicate (copied blocks; \
                       arrives a little later). Cached: commits and closed efforts are analyzed once."
    )]
    async fn ensure_change(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<EnsureChangeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.change.ensure",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Ask a model a typed question about some text, cheaply: yes/no (`noul`, \
                       returns the probability of yes), `choice` among options, or `score` on \
                       ordered levels, each with probabilities, on the `decide` role. Good for \
                       second opinions like \"is this diff risky?\". Recorded: the same \
                       questions on the same text are answered once (`cached`). Fails if the \
                       role has no model; the person assigns one in Settings → AI."
    )]
    async fn ai_decide(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<AiDecideParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.ai.decide",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Summarize text with oxplow's `summarize` role (a model the person \
                       configured in Settings → AI), optionally with a focus. Useful for long \
                       logs or documents you don't need verbatim. Recorded: the same text and \
                       focus are summarized once."
    )]
    async fn ai_summarize(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<AiSummarizeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.ai.summarize",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Before installing (`git_url`) or updating (`name`) an extension: clone it \
                       and report what it would bring in, installing nothing: the extension as it \
                       would load (lenses, sources with the programs they run, the hosts they \
                       reach and the credentials they read, advisories, collectors; `errors` block \
                       the install), the commit `sha`, and `problems` a dry run of its lenses \
                       found. Show the person what it declares and get their go-ahead, then \
                       pass `sha` as `reviewed_sha` to the `oxplow.extension.install` / `oxplow.extension.update` commands \
                       (a person approves them)."
    )]
    async fn review_extension(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ReviewExtensionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.extension.review",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Show the person an answer in their thread — a table, chart, tree, list of \
                       refs … — instead of pasting it as text: an existing lens with params, or \
                       a lens of your own (`spec`). It renders beside the conversation, live, \
                       with a \"Keep this\" that turns it into a lens they can reopen and share. \
                       Returns the answer's ref and its text rendering (what they see). Its SQL \
                       has `query_sql`'s rights: read-only, over the `v_*` models."
    )]
    async fn show_lens(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ShowLensParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let caller = caller_of(&extensions);
        let actor = self.verified_actor(&caller).await?;
        let out = self
            .services
            .commands
            .run(
                &actor,
                oxplow_app::commands::lens::SHOW,
                serde_json::json!({ "lens": p.lens, "spec": p.spec, "params": p.params }),
                false,
            )
            .await
            .map_err(command_error)?;
        let answer = out.result["answer"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let id: i64 =
            oxplow_domain::refs::build::parse_as(&answer, oxplow_domain::refs::kind::ANSWER)
                .ok()
                .ok_or_else(|| internal("oxplow.lens.show returned no answer"))?;
        let (run, text) = oxplow_app::commands::lens::text_answer(&self.services, id)
            .await
            .map_err(domain_err)?;
        json_result(&serde_json::json!({ "answer": answer, "title": run.lens.title, "text": text }))
    }

    #[tool(
        description = "Run a lens and read what the human sees on its page: by default its text \
                       rendering (a table, a chart's series, a grid's children) plus the resolved \
                       params, columns, row count and alert; `format: \"json\"` returns the raw \
                       rows instead. Override params by name; the rest use defaults."
    )]
    async fn run_lens(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<RunLensInput>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.lens.run",
            serde_json::to_value(&params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Run one of a lens's actions (see `read` of `lens:<ext>/<slug>` → `actions`): the command it \
                       declares, run as the lens acting for you — so the same rules apply as if \
                       you ran the command yourself (a command you can't run, or one that needs \
                       a person's confirmation, is refused). A row action (`row: true`) needs the \
                       `row` it runs on. Returns the command's outcome."
    )]
    async fn run_lens_action(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<RunLensActionInput>,
    ) -> Result<CallToolResult, McpError> {
        self.run_command_as(
            &caller_of(&extensions),
            RunCommandParams {
                id: "oxplow.lens.run_action".into(),
                input: serde_json::to_value(&params.0).map_err(internal)?,
            },
        )
        .await
    }

    #[tool(
        description = "Check an extension after editing it (the same report as `oxplow extension \
                       check`): manifest and lifecycle errors with file:line, unresolved \
                       cross-references, plus a dry run of every lens and advisory (SQL \
                       errors, `columns` keys the query doesn't return). `ok: true` means it \
                       works; `warnings` are worth fixing but don't block."
    )]
    async fn validate_extension(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ValidateExtensionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.extension.validate",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Preview a whole extension in a throwaway oxplow: what `oxplow extension \
                       test` runs, never touching this project's data. It checks the extension, \
                       activates it with the project's other extensions, and runs every intent \
                       example that has a fixture — lenses, collectors, commands, effects, \
                       events, jobs and components — with model calls answered by the fixture's \
                       replies, never a real provider. Returns `{ ok, errors, warnings, ran, … }`; \
                       run it after `validate_extension` and before handing the extension over."
    )]
    async fn preview_extension(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ValidateExtensionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.extension.preview",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Read what a ref names: its model's row (every column), what sits beside \
                       it (a work item's links and comments, a commit's or an effort's files) \
                       and, for a body no model holds, its content. `file:src/a.rs` reads the \
                       working tree; `file:src/a.rs@git:HEAD#L10-40` a revision's lines \
                       (`@snap:<id>` a local-history snapshot); `event:<id>#output` (or \
                       `#input`, `#prompt`) one of your stream's events' stored bodies; \
                       `lens:<ext>/<slug>` a lens's definition; `stream` reads a file or lens in \
                       another stream's worktree. `ai_call:<id>#request` (or `#response`) a \
                       recorded model call's bodies. Any `ref` a model's column \
                       holds reads the same way (`work_item:…`, `commit:…`, `effort:…`, \
                       `thread:…`, `wiki:…`). Content stops at 64 KB, saying so."
    )]
    async fn read(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ReadInput>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.records.read",
            serde_json::to_value(&params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Run ONE read-only SQL statement (`SELECT`/`WITH`) over the published \
                       models — the `v_*` views lenses and the UI read (`v_model` lists them; \
                       physical tables are refused). Joins across views are fine. Metrics read \
                       as columns of a grid: `SELECT bucket, zone, MEASURE('<metric key>') FROM \
                       metric_grid('day'|'week'|'month'[, '<dimension>'])` (keys in \
                       v_metric_spec, dimensions in v_dimension). Positional params `?1`, `?2`, … \
                       bind from `params`. Returns `{columns, rows, truncated, reads, generation, \
                       coverage}`; rows are positional arrays; `truncated` means more rows \
                       existed than `limit`. `coverage` names the collector behind each \
                       collected model it read and how its last run stands (`current`, \
                       `stale`, `failed`, `disabled`, `needs_approval`, `never`): a complete \
                       result over a failed collector is still missing data. Writes, PRAGMA, ATTACH and multiple statements are \
                       rejected; queries time out after 5s. `v_model_column` documents every \
                       column."
    )]
    async fn query_sql(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<QuerySqlInput>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.sql.query",
            serde_json::to_value(&params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Site-wide BM25 search across tasks, comments, notes, wiki pages, and \
                       per-stream file contents. Tokens match as stemmed prefixes. `stream_id` \
                       scopes file/stream-bound hits to one worktree (wiki etc. always included); \
                       omit it to search everything. `kinds` optionally restricts to \
                       task|comment|note|wiki|file. Returns hits ranked best-first."
    )]
    async fn search(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<SiteSearchInput>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.search.find",
            serde_json::to_value(&params.0).map_err(internal)?,
        )
        .await
    }

    // ---------- git (read) ----------
    //
    // Thin mirrors of the IPC reads over the VCS (and the git provider), so the
    // agent inspects the worktree through the same path the UI does (consistent
    // results, snapshot/event hooks) instead of shelling out to raw `git`.
    // `stream_id` is optional — omit to target the current/primary worktree.
    // Mutations (commit/push/merge/…) are the `oxplow.vcs.*` commands
    // (`run_command`), not tools.

    #[tool(
        description = "Blame a file: the revision, author and time that last changed each \
                          line, at `revision` (default `working`; an uncommitted line names no \
                          revision)."
    )]
    async fn vcs_blame(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<BlameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.vcs.blame",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "What changed between two versions of the tree, per file with line \
                          counts. Versions are `working`, `snap:<id>` or `git:<rev>`; `from` \
                          absent diffs against the empty tree. `since_fork: true` compares `to` \
                          with where it forked from `from` — a branch's own changes against \
                          `git:main`."
    )]
    async fn diff(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<DiffParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.vcs.diff",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    // ---------- snapshots / local history (read + restore) ----------
    //
    // The listings are `v_snapshot`, `v_snapshot_op` and `v_snapshot_file`
    // through `query_sql`; these read file content. Ids are honest: a
    // `snapshot_id` is a whole capture, a `file_snapshot_id` one captured
    // file row; reads share `oxplow_app::snapshot_files`.

    // ---------- work items ----------
    //
    // Read through the work-item interface, whichever list is active
    // (`oxplow_app::work_item_reads`); with none, they're empty. Writes
    // are the `work_item.*` commands, through `run_command`.

    // ---------- collection (test runs + diff coverage) ----------

    // ---------- comments ----------

    #[tool(
        description = "List comments — threaded annotations the user anchored to a text selection \
                       in a page (wiki body, code file line, task detail). `id` is a thread \
                       (`thr…`) or stream (`str…`) id; `scope` is inferred from its prefix, so \
                       usually pass only `id`. `status` filters: \"all\" (default), \"open\", or \
                       \"needs_response\" (open follow-ups whose latest message isn't yours — what \
                       the user wants you to act on). Each result carries the anchored `quote`, \
                       the message thread, and `intent` (note vs followup). Reply with \
                       `run_command oxplow.knowledge.reply_comment { comment, body }`; resolve with \
                       `run_command oxplow.knowledge.update_comment { comment, status: \"resolved\" }`."
    )]
    async fn list_comments(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ListCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.knowledge.comments",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(description = "For one wiki page file ref, return the unified diff \
                       between the snapshot the ref was pinned to and the \
                       file's CURRENT on-disk content — so you can read just \
                       what drifted instead of re-opening the whole file. \
                       Find the drifted refs with `query_sql` over \
                       `v_knowledge_ref` (`stale = 1`; per page, \
                       `v_knowledge_page.stale_ref_count`), then call this \
                       per ref. Returns \
                       `{ slug, path, pinned_snapshot_id, status, \
                       unified_diff, truncated }`; `status` is one of \
                       drifted | unchanged | not_a_ref | no_pin | binary | no_contents \
                       (the pin was taken by 'Track changes only', which keeps no text).")]
    async fn wiki_ref_drift(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<WikiRefDriftParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.knowledge.drift",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    // ---------- task orchestration ----------

    #[tool(
        description = "The commands this agent may run through `run_command`. With no \
                       arguments: every command's `id` and one-line `summary`. With `id` (one \
                       command) or `prefix` (an area, `oxplow.work_item.`): each with its \
                       `input_schema`, an `example` input that fits it, whether a person \
                       `confirm`s it and whether it is `undoable`. Read a command this way \
                       before its first run. Commands are the one write path: every run is \
                       validated, policy-checked, audited and logged as `command.executed`."
    )]
    async fn list_commands(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ListCommandsParams>,
    ) -> Result<CallToolResult, McpError> {
        let actor = caller_of(&extensions).actor();
        let offered = self.services.commands.list(&actor);
        let p = params.0;
        if p.id.is_none() && p.prefix.is_none() {
            let index: Vec<CommandIndexEntry> = offered
                .into_iter()
                .map(|spec| CommandIndexEntry {
                    summary: first_sentence(&spec.summary).to_string(),
                    id: spec.id,
                })
                .collect();
            return json_result(&index);
        }
        let views: Vec<CommandView> = offered
            .iter()
            .filter(|spec| p.id.as_deref().is_none_or(|id| spec.id == id))
            .filter(|spec| {
                p.prefix
                    .as_deref()
                    .is_none_or(|pre| spec.id.starts_with(pre))
            })
            .map(CommandView::of)
            .collect();
        if views.is_empty() {
            return Err(McpError::invalid_params(
                no_command_message(&offered, p.id.as_deref(), p.prefix.as_deref()),
                None,
            ));
        }
        json_result(&views)
    }

    #[tool(
        description = "Run a command by its id with its input (`list_commands { id }` shows its \
                       schema and an example input). Returns \
                       `{ result, audit_id, event_id, inverse? }`. Invalid input names the \
                       failing field; a denied command says why. A command that needs a \
                       person's confirmation is not run: it is recorded as a proposal and \
                       this returns `{ proposal, message }` — tell the person it waits for \
                       them in this thread and in Alerts, and don't run it again; its \
                       `decision` is in `v_command_proposal`. \
                       Requires the connection's thread identity: an anonymous connection may \
                       not write."
    )]
    async fn run_command(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<RunCommandParams>,
    ) -> Result<CallToolResult, McpError> {
        let caller = caller_of(&extensions);
        self.run_command_as(&caller, params.0).await
    }

    #[tool(
        description = "Where the symbol at a position is defined — typed locations \
                       (`{ path, range: { start: { line, col }, end } }`, 1-based; paths \
                       workspace-relative, absolute outside it), from the file's language \
                       server. `v_lsp_server` says which languages have one."
    )]
    async fn code_definition(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodePositionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.definition",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Every reference to the symbol at a position (typed locations, \
                       1-based), the declaration included unless `include_declaration` is \
                       false."
    )]
    async fn code_references(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodeReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.references",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "The hover for a position — the symbol's type/signature and docs, as \
                       markdown (`{ contents, range }`), or null."
    )]
    async fn code_hover(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodePositionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.hover",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "The symbols a file declares (functions, classes, methods, modules, …), \
                       nested ones naming their container, as `{ source, symbols }`. `source` \
                       says what answered: `language_server` (any configured language whose \
                       server runs; `{ name, kind, container, location }`) or `tree_sitter` \
                       (no server serves the file, so oxplow's own offline analysis reads the \
                       file's live content, the uncommitted edits included; `{ kind, name, \
                       containerPath, startLine, endLine }`, empty for an unsupported or \
                       unparseable file). `v_symbol` holds the stored symbols of captured files."
    )]
    async fn code_symbols(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodeSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.symbols",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "Find symbols across the workspace by name (fuzzy, server-defined) from \
                       one language's running server — typed symbols with their locations. Use \
                       to find a definition by name without a file/position. Needs a running \
                       server (`v_lsp_server`); without one, search the stored symbols in \
                       `v_symbol`."
    )]
    async fn code_workspace_symbols(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodeWorkspaceSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.workspace_symbols",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }

    #[tool(
        description = "The symbol at a position's callers (`direction: incoming`) or callees \
                       (`outgoing`): each `{ symbol, at }` — the caller/callee and where the \
                       calls are. [] when the position has no call-hierarchy item."
    )]
    async fn code_call_hierarchy(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<CodeCallHierarchyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_command(
            &caller_of(&extensions),
            "oxplow.code.call_hierarchy",
            serde_json::to_value(params.0).map_err(internal)?,
        )
        .await
    }
}

/// Tools that only READ state — annotated `read_only_hint` so a client can
/// auto-approve them instead of prompting (tsk203). Kept as an explicit set
/// (rather than 100 per-`#[tool]` annotations) so the read/write split lives in
/// one reviewable place; `read_write_split_covers_every_tool` fails if a new
/// tool isn't classified here or in [`WRITE_TOOLS`].
const READ_ONLY_TOOLS: &[&str] = &[
    "get_skill",
    "get_open_page",
    "run_lens",
    "validate_extension",
    "preview_extension",
    "query_sql",
    "search",
    "vcs_blame",
    "diff",
    "read",
    "list_comments",
    "wiki_ref_drift",
    "list_commands",
    "code_definition",
    "code_hover",
    "code_references",
    "code_symbols",
    "code_workspace_symbols",
    "code_call_hierarchy",
];

/// Tools that MUTATE state — deliberately left without a `read_only_hint` (they
/// stay gated). Not consumed at runtime; exists so the classification test can
/// prove every registered tool is accounted for (read XOR write).
#[cfg(test)]
const WRITE_TOOLS: &[&str] = &[
    // P8.A10: an agent writes project records only through the bus. What
    // else isn't read-only, and why each isn't a write path of its own:
    //
    // The one write path: every command, audited to the calling thread.
    "run_command",
    // A lens's action runs its command, as the lens acting for the caller.
    "run_lens_action",
    // The `oxplow.lens.show` command (records the answer in the caller's thread).
    "show_lens",
    // The `oxplow.collector.sync` command.
    // A derived cache: the change's analysis, recomputed from the VCS on
    // demand (ipc-and-stores.md, "what stays off the bus").
    "ensure_change",
    // Runs a source's program; stores nothing.
    "preview_collector",
    // Clones into .oxplow/tmp; stores nothing.
    "review_extension",
    // A model call, recorded as the computation it was (`ai_call`).
    "ai_decide",
    "ai_summarize",
];

/// Stamp `read_only_hint = true` on tools in [`READ_ONLY_TOOLS`], leaving any
/// existing annotation fields intact. Shared by `list_tools` and its test.
fn stamp_read_only_hints(tools: Vec<Tool>) -> Vec<Tool> {
    tools
        .into_iter()
        .map(|mut tool| {
            if READ_ONLY_TOOLS.contains(&tool.name.as_ref()) {
                let ann = tool.annotations.take().unwrap_or_default().read_only(true);
                tool.annotations = Some(ann);
            }
            tool
        })
        .collect()
}

impl ServerHandler for OxplowMcp {
    fn get_info(&self) -> ServerConfig {
        // `ServerConfig` (née `ServerInfo`, renamed in rmcp 3) is
        // #[non_exhaustive], so it can't be built with struct-expression
        // syntax (not even with a `..Default::default()` tail). Start from
        // the default and assign.
        let mut info = ServerConfig::default();
        info.instructions = Some(
            "Oxplow knows this project's work and code: work items, efforts, \
             test runs and coverage, commits, metrics and the wiki, readable \
             with `query_sql` over the read-only `v_*` models (`v_model` lists \
             them). Every write is a command (`run_command`).\n\n\
             Answering the person: when they ask to see, list, compare or \
             track something about the work or the code (\"show me\", \
             \"which files\", \"how well\"), answer from oxplow's data and \
             show it with `show_lens` (an existing lens with params, or your \
             own `spec`: a table, chart or tree). It lands beside the \
             conversation, live, with Keep This. Keep the prose around it \
             short, and don't repeat the table as text."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    // These three methods replace what `#[tool_handler]` would generate. We
    // hand-roll them only so `list_tools` can stamp `read_only_hint` on the
    // read-tool family (tsk203); everything else — including rmcp 3's cache
    // hints — mirrors the macro's output (rmcp-macros `tool_handler.rs`), so
    // re-diff against it on an rmcp bump.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: stamp_read_only_hints(self.tool_router.list_all()),
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(CacheScope::Public),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// Names of every tool registered on the MCP surface.
///
/// Used by the cross-surface parity test in `oxplow-surface-parity` to
/// enforce that the agent (MCP) and UI (Tauri IPC) adapters stay in sync.
/// `tool_router()` is generated by `#[tool_router]` and takes no `self`,
/// so this needs no `Services` instance, no async runtime, and no I/O.
pub fn registered_tool_names() -> Vec<String> {
    OxplowMcp::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.into_owned())
        .collect()
}

fn internal<E: std::fmt::Display>(e: E) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// A domain failure as a tool error: bad input is the caller's.
fn domain_err(e: oxplow_domain::DomainError) -> McpError {
    match e {
        oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
        other => internal(other),
    }
}

impl OxplowMcp {
    /// A read command (`oxplow.records.read`, …) as `caller`, for the tool
    /// that is now a thin call to it: an anonymous connection reads too
    /// (it may not write), and the command's result is the tool's — text
    /// as is, anything else as JSON.
    async fn read_command(
        &self,
        caller: &McpCaller,
        id: &str,
        input: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        let actor = if caller.thread_id.is_some() {
            self.verified_actor(caller).await?
        } else {
            caller.actor()
        };
        let outcome = self
            .services
            .commands
            .run(&actor, id, input, false)
            .await
            .map_err(command_error)?;
        match outcome.result {
            serde_json::Value::String(text) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            other => json_result(&other),
        }
    }

    /// `run_command` for a known caller. Refuses an anonymous connection:
    /// a write with no actor behind it is not audited to anyone.
    pub async fn run_command_as(
        &self,
        caller: &McpCaller,
        params: RunCommandParams,
    ) -> Result<CallToolResult, McpError> {
        let actor = self.verified_actor(caller).await?;
        let outcome = match self
            .services
            .commands
            .run(&actor, &params.id, params.input, false)
            .await
        {
            Ok(outcome) => outcome,
            // Kept for a person: the agent's request is recorded and needs
            // nothing more from it.
            Err(oxplow_domain::CommandError::Proposed {
                proposal,
                preview,
                supersedes,
            }) => {
                // `kind` tells it apart from a run's outcome.
                return json_result(&serde_json::json!({
                    "kind": "proposed",
                    "proposal": proposal,
                    "message": proposed_message(&preview.command, &proposal, &supersedes),
                }));
            }
            Err(err) => return Err(command_error(err)),
        };
        // A write may have opened or closed an effort (`oxplow.effort.open`,
        // `oxplow.work_item.transition`): let its snapshot pin land before the
        // agent's next step.
        if outcome.audit_id.is_some() {
            self.services.efforts.settle_lifecycle().await;
        }
        json_result(&outcome)
    }

    async fn verified_actor(&self, caller: &McpCaller) -> Result<oxplow_domain::Actor, McpError> {
        use oxplow_domain::stores::ThreadStore as _;
        let Some(thread_id) = caller.thread_id else {
            return Err(McpError::invalid_params(ANONYMOUS_WRITE, None));
        };
        let thread = self
            .services
            .thread_store
            .get(&thread_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("unknown thread `{thread_id}` in this connection's identity"),
                    None,
                )
            })?;
        if let Some(stream) = caller.stream_id {
            if stream != thread.stream_id {
                return Err(McpError::invalid_params(
                    format!(
                        "thread `{thread_id}` belongs to stream `{}`, not `{stream}`",
                        thread.stream_id
                    ),
                    None,
                ));
            }
        }
        Ok(oxplow_domain::Actor::Agent {
            session_id: caller.session_id,
            thread_id: Some(thread_id),
            stream_id: Some(thread.stream_id),
        })
    }
}

/// A command-bus refusal as the MCP error the agent can act on: invalid
/// input and denials are the caller's to fix; a needed confirmation says
/// so; a handler failure is internal.
fn command_error(err: oxplow_domain::CommandError) -> McpError {
    use oxplow_domain::CommandError as E;
    match &err {
        E::Unknown { .. } | E::Invalid { .. } | E::Denied { .. } => {
            McpError::invalid_params(err.to_string(), None)
        }
        E::NeedsConfirmation { preview } => McpError::invalid_params(
            format!(
                "`{}` needs a person's confirmation; ask them to run it (input: {})",
                preview.command, preview.input
            ),
            None,
        ),
        E::Proposed {
            proposal,
            preview,
            supersedes,
        } => McpError::invalid_params(
            proposed_message(&preview.command, proposal, supersedes),
            None,
        ),
        E::Failed { .. } | E::Unavailable { .. } | E::Busy { .. } => internal(err.to_string()),
    }
}

/// What an agent is told when its run waits for a person (`proposal:N`).
fn proposed_message(command: &str, proposal: &str, supersedes: &[String]) -> String {
    let replaces = if supersedes.is_empty() {
        String::new()
    } else {
        format!(
            " It replaces {}, which no longer waits.",
            supersedes.join(", ")
        )
    };
    format!(
        "`{command}` needs a person's approval; it is recorded as {proposal} and waits for \
         them in this thread and in Alerts (and on the setting's row in Settings).{replaces} \
         Tell the person; don't run it again. To see what they decided, read its `decision` \
         from `v_command_proposal` (`WHERE ref = '{proposal}'`)."
    )
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

/// Convenience wrapper: spawn the server on stdio.
pub async fn serve_stdio(services: Arc<Services>) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;
    let server = OxplowMcp::new(services);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "a test seeds the database through its stores"
    )]

    use super::*;
    use oxplow_app::commands::agent_reads::ai::{AiQuestionParam, AI_TEXT_MAX};

    /// tsk203: every registered tool must be classified read XOR write, so a new
    /// tool can't slip in un-annotated (a write mis-marked read is a safety bug;
    /// a read left un-marked just re-introduces the permission prompt).
    /// `ensure_change` stores the analysis and starts a duplicate scan, so
    /// it isn't read-only (tsk371).
    #[test]
    fn ensure_change_is_not_hinted_read_only() {
        assert!(!READ_ONLY_TOOLS.contains(&"ensure_change"));
        assert!(WRITE_TOOLS.contains(&"ensure_change"));
    }

    /// P8.A10: an agent writes records through `run_command`; the named
    /// write tools are gone and stay gone.
    #[test]
    fn read_write_split_covers_every_tool() {
        use std::collections::HashSet;
        let registered: HashSet<String> = registered_tool_names().into_iter().collect();
        let read: HashSet<String> = READ_ONLY_TOOLS.iter().map(|s| s.to_string()).collect();
        let write: HashSet<String> = WRITE_TOOLS.iter().map(|s| s.to_string()).collect();

        let overlap: Vec<_> = read.intersection(&write).collect();
        assert!(
            overlap.is_empty(),
            "tool in BOTH read and write sets: {overlap:?}"
        );

        let unclassified: Vec<_> = registered
            .difference(&read.union(&write).cloned().collect())
            .cloned()
            .collect();
        assert!(
            unclassified.is_empty(),
            "new tool(s) not classified in READ_ONLY_TOOLS or WRITE_TOOLS: {unclassified:?}",
        );

        let stale: Vec<_> = read
            .union(&write)
            .filter(|t| !registered.contains(*t))
            .cloned()
            .collect();
        assert!(
            stale.is_empty(),
            "classified tool no longer registered: {stale:?}"
        );
    }

    /// The stamping must produce `read_only_hint = true` for exactly the read
    /// tools and leave writes unmarked — this is what the client keys on.
    #[test]
    fn stamping_marks_exactly_the_read_tools() {
        let tools = stamp_read_only_hints(OxplowMcp::tool_router().list_all());
        for t in &tools {
            let is_read = READ_ONLY_TOOLS.contains(&t.name.as_ref());
            let marked = t
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
                .unwrap_or(false);
            assert_eq!(marked, is_read, "tool {} read_only_hint mismatch", t.name);
        }
    }

    async fn boot() -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        boot_with_config("").await
    }

    /// `boot().await` with `project_yaml` written to `.oxplow/project.yaml`
    /// first, so the booted `Services` picks it up (e.g. a
    /// `generated:` block that shapes the workspace filter).
    async fn boot_with_config(project_yaml: &str) -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        let project = tempfile::tempdir().unwrap();
        if !project_yaml.is_empty() {
            std::fs::create_dir_all(project.path().join(".oxplow")).unwrap();
            std::fs::write(project.path().join(".oxplow/project.yaml"), project_yaml).unwrap();
        }
        // ensure_primary requires a real git repo.
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let services = Services::in_memory(project.path()).await.unwrap();
        let server = OxplowMcp::new(services.clone());
        (project, services, server)
    }

    /// Pull the first text block out of an MCP CallToolResult. Most
    /// of our handlers return a single JSON-encoded blob.
    fn text_payload(result: CallToolResult) -> String {
        for c in &result.content {
            if let Some(text) = c.as_text() {
                return text.text.clone();
            }
        }
        panic!("CallToolResult had no text content");
    }

    /// A task filed as a person through `oxplow.work_item.create` — on
    /// `thread`'s list, else the backlog: its id.
    async fn file_task(
        services: &Services,
        thread: Option<oxplow_domain::ThreadId>,
        title: &str,
    ) -> oxplow_tasks::TaskId {
        let mut input = serde_json::json!({ "title": title });
        if let Some(t) = thread {
            input["thread"] = serde_json::Value::String(t.to_string());
        }
        let out = services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                input,
                false,
            )
            .await
            .unwrap();
        oxplow_tasks::task_of_work_item_ref(out.result["ref"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn server_constructs() {
        let (_proj, _svc, _server) = boot().await;
    }

    #[tokio::test]
    async fn get_open_page_reports_what_the_human_sees() {
        use oxplow_domain::stores::ThreadStore;
        let (proj, services, server) = boot().await;
        let stream = services.streams.list_streams().await.unwrap()[0].clone();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()[0]
            .id;
        let tid = thread.to_string();

        let none: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .get_open_page(
                    rmcp::model::Extensions::default(),
                    Parameters(OpenPageInput {
                        thread_id: tid.clone(),
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(none["open"], serde_json::Value::Null);

        let p = proj.path().join("oxplow/extensions/demo/lenses");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            proj.path().join("oxplow/extensions/demo/extension.yaml"),
            "manifest: 2\nname: demo\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            p.join("kinds.yaml"),
            "title: Kinds\nparams:\n  - { name: kind, default: worktree }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        )
        .unwrap();
        services
            .extension_activation
            .activate(services.clone())
            .await
            .unwrap();
        services.thread_runtime.set_open_page(
            &thread,
            Some(oxplow_app::thread_runtime::OpenPage {
                page_id: "lens:demo/kinds".into(),
                kind: "lens".into(),
                detail_json: Some(r#"{"lensId":"demo/kinds","params":{"kind":"primary"}}"#.into()),
                reported_at: oxplow_domain::Timestamp::now(),
            }),
        );
        let open: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .get_open_page(
                    rmcp::model::Extensions::default(),
                    Parameters(OpenPageInput { thread_id: tid }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(open["open"]["pageId"], "lens:demo/kinds");
        assert_eq!(open["open"]["detail"]["params"]["kind"], "primary");
        // The lens is re-run with the human's params, not the defaults,
        // and read as its text.
        assert_eq!(open["lens"]["params"]["kind"], "primary");
        assert_eq!(open["lens"]["rowCount"], 1);
        assert!(
            open["lens"]["text"]
                .as_str()
                .unwrap()
                .contains("| primary |"),
            "{}",
            open["lens"]
        );
        assert!(open["lens"].get("result").is_none());
    }

    #[tokio::test]
    async fn extension_and_lens_tools() {
        let (proj, services, server) = boot().await;
        let root = proj.path();
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w(
            "oxplow/extensions/demo/extension.yaml",
            "manifest: 2\nname: demo\nintent:\n  purpose: test\nui:\n  slots:\n    - { slot: effort.review.details, lens: review }\n",
        );
        w(
            "oxplow/extensions/demo/lenses/review.yaml",
            "title: Review\nparams:\n  - { name: effort_id }\nquery: SELECT :effort_id AS effort\n",
        );
        w(
            "oxplow/extensions/demo/lenses/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        );
        w(
            "oxplow/extensions/demo/lenses/broken.yaml",
            "title: Broken\nquery: SELECT x FROM v_nope\n",
        );
        services
            .extension_activation
            .activate(services.clone())
            .await
            .unwrap();

        // A lens's definition is what `read` gives for its ref.
        let read: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .read(
                    rmcp::model::Extensions::new(),
                    Parameters(ReadInput {
                        reference: "lens:demo/streams".into(),
                        stream: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        let lens: serde_json::Value =
            serde_json::from_str(read["content"].as_str().unwrap()).unwrap();
        assert!(lens["query"].as_str().unwrap().contains("v_stream"));

        let run = |format: Option<&str>| {
            let format = format.map(str::to_string);
            let server = &server;
            async move {
                serde_json::from_str::<serde_json::Value>(&text_payload(
                    server
                        .run_lens(
                            rmcp::model::Extensions::new(),
                            Parameters(RunLensInput {
                                id: "demo/streams".into(),
                                params: Some(
                                    [("kind".to_string(), serde_json::json!("primary"))]
                                        .into_iter()
                                        .collect(),
                                ),
                                stream_id: None,
                                thread_id: None,
                                format,
                            }),
                        )
                        .await
                        .unwrap(),
                ))
                .unwrap()
            }
        };
        // By default an agent reads the text rendering, not the rows (tsk576).
        let text = run(None).await;
        assert!(text.get("result").is_none(), "{text}");
        assert_eq!(text["rowCount"], 1);
        assert!(
            text["text"].as_str().unwrap().contains("| primary |"),
            "{text}"
        );
        let json = run(Some("json")).await;
        assert_eq!(json["result"]["rows"], serde_json::json!([["primary"]]));

        let err = server
            .read(
                rmcp::model::Extensions::new(),
                Parameters(ReadInput {
                    reference: "lens:demo/nope".into(),
                    stream: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("lens:demo/nope"), "{err:?}");

        let v: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .validate_extension(
                    rmcp::model::Extensions::new(),
                    Parameters(ValidateExtensionParams {
                        name: "demo".into(),
                        stream_id: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert!(
            v["errors"][0].as_str().unwrap().contains("demo/broken"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn semantic_layer_query_and_schema_tools() {
        let (_proj, _services, server) = boot().await;
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .query_sql(
                    rmcp::model::Extensions::default(),
                    Parameters(QuerySqlInput {
                        sql: "SELECT kind FROM v_stream WHERE kind = ?1".into(),
                        params: Some(vec![serde_json::json!("primary")]),
                        limit: Some(5),
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["columns"], serde_json::json!(["kind"]));
        assert_eq!(out["rows"], serde_json::json!([["primary"]]));

        let err = server
            .query_sql(
                rmcp::model::Extensions::default(),
                Parameters(QuerySqlInput {
                    sql: "DELETE FROM task".into(),
                    params: None,
                    limit: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("read-only") || err.message.contains("SELECT"),
            "{err:?}"
        );

        // The catalog is SQL too: every model's columns are documented.
        let cols: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .query_sql(
                    rmcp::model::Extensions::default(),
                    Parameters(QuerySqlInput {
                        sql: "SELECT name FROM v_model_column WHERE view = 'v_work_item'".into(),
                        params: None,
                        limit: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert!(cols["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r[0] == "state"));
    }

    #[tokio::test]
    async fn list_comments_enriches_primary_and_context() {
        let (_proj, services, server) = boot().await;
        let stream = services.streams.list_streams().await.unwrap()[0].id;

        // A task to use as the primary target, and another as a context
        // ancestor (e.g. an epic the row sat under).
        let primary_task = file_task(&services, None, "Primary item").await;
        let parent_task = file_task(&services, None, "Parent epic").await;

        services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.knowledge.add_comment",
                serde_json::json!({
                    "stream": format!("stream:{stream}"),
                    "target": { "kind": "work_item", "id": format!("oxplow:{primary_task}") },
                    "quote": "the highlighted text",
                    "selectors_json": "[]",
                    "context_chain": [{ "kind": "work_item", "id": format!("oxplow:{parent_task}") }],
                    "referenced_refs": [{ "kind": "file", "id": "src/app.rs" }],
                    "intent": "followup",
                    "body": "what about this?",
                }),
                false,
            )
            .await
            .unwrap();

        let r = server
            .list_comments(
                rmcp::model::Extensions::new(),
                Parameters(ListCommentsParams {
                    scope: Some("stream".into()),
                    id: Some(stream.to_string()),
                    status: None,
                }),
            )
            .await
            .unwrap();
        let body = text_payload(r);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let row = &parsed.as_array().unwrap()[0];

        // Primary target resolved to the task title.
        assert_eq!(row["primary"]["kind"], "work_item");
        assert_eq!(row["primary"]["title"], "Primary item");
        // Context chain ancestor resolved.
        assert_eq!(row["context_chain"][0]["title"], "Parent epic");
        // Referenced file ref present but bare (no first-class label).
        assert_eq!(row["referenced"][0]["kind"], "file");
        assert_eq!(row["referenced"][0]["id"], "src/app.rs");
        assert!(row["referenced"][0]["title"].is_null());
        // The raw thread still travels under `thread`.
        assert_eq!(row["thread"]["comment"]["quote"], "the highlighted text");
    }

    #[tokio::test]
    async fn lsp_no_config_error_is_self_describing_for_agents() {
        let (_proj, services, server) = boot().await;
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let err = server
            .code_hover(
                rmcp::model::Extensions::new(),
                Parameters(CodePositionParams {
                    stream_id,
                    path: "src/x.rs".into(),
                    line: 1,
                    col: 1,
                }),
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("rust-analyzer"), "got: {msg}");
        assert!(msg.contains("oxplow.lsp.install_server"), "got: {msg}");
        assert!(msg.contains("project.yaml"), "got: {msg}");
    }

    #[tokio::test]
    async fn get_info_advertises_tool_capability() {
        let (_proj, _svc, server) = boot().await;
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }

    /// tsk1033: asked to "show me", an agent printed text. The
    /// instructions every harness reads say to show an answer in the
    /// person's thread.
    #[tokio::test]
    async fn get_info_instructions_say_to_show_answers_in_the_thread() {
        let (_proj, _svc, server) = boot().await;
        let instructions = server.get_info().instructions.unwrap_or_default();
        assert!(instructions.contains("show_lens"), "{instructions}");
        assert!(instructions.contains("query_sql"), "{instructions}");
    }

    #[tokio::test]
    async fn get_info_instructions_have_no_repo_specific_context_path() {
        // The instructions ship to every downstream agent — they must not
        // point at this repo's own `.context/` docs, which don't exist
        // in a user's project.
        let (_proj, _svc, server) = boot().await;
        let info = server.get_info();
        let instructions = info.instructions.unwrap_or_default();
        assert!(
            !instructions.contains(".context/"),
            "leaked repo path: {instructions}"
        );
    }

    /// An agent that can't discover skill files reads them by name
    /// (tsk376); it's read-only.
    #[tokio::test]
    async fn get_skill_returns_a_skill_body() {
        let (_proj, _svc, server) = boot().await;
        let r = server
            .get_skill(
                rmcp::model::Extensions::default(),
                Parameters(GetSkillParams {
                    name: "oxplow-extension".into(),
                    reference: None,
                }),
            )
            .await
            .unwrap();
        assert!(text_payload(r).contains("# Building oxplow lenses"));
        let err = server
            .get_skill(
                rmcp::model::Extensions::default(),
                Parameters(GetSkillParams {
                    name: "nope".into(),
                    reference: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("oxplow-extension"), "{err:?}");
        // A skill's reference files are read by name too.
        let core = oxplow_app::capabilities::agent_text(&_svc);
        let skill = core
            .skills
            .iter()
            .find(|s| !s.references.is_empty())
            .expect("a skill with reference files");
        let reference = &skill.references[0];
        let r = server
            .get_skill(
                rmcp::model::Extensions::default(),
                Parameters(GetSkillParams {
                    name: skill.name.clone(),
                    reference: Some(reference.name.clone()),
                }),
            )
            .await
            .unwrap();
        assert_eq!(text_payload(r), reference.body);
        let err = server
            .get_skill(
                rmcp::model::Extensions::default(),
                Parameters(GetSkillParams {
                    name: skill.name.clone(),
                    reference: Some("nope".into()),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains(&reference.name), "{err:?}");
        assert!(READ_ONLY_TOOLS.contains(&"get_skill"));
    }

    /// An agent previews a source from its stream's worktree: rows back,
    /// nothing stored (tsk377).
    #[tokio::test]
    async fn preview_collector_returns_rows_without_storing() {
        let (proj, svc, server) = boot().await;
        let ext = proj.path().join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nintent:\n  purpose: test\ncollectors:\n  - id: one\n    runtime: jaq\n    entry: one.jq\n    input: \"SELECT 1 AS n\"\n    entities:\n      - { name: nums, key: n, columns: { n: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("one.jq"), "{entities: {nums: .rows}}").unwrap();
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        let r = server
            .preview_collector(
                rmcp::model::Extensions::new(),
                Parameters(PreviewCollectorParams {
                    owner: "work".into(),
                    id: "one".into(),
                    stream_id: None,
                    event: None,
                }),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(v["entities"][0]["rows"], serde_json::json!([[1]]));
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
    }

    /// A fact collector previews through the fact engine: the facts it
    /// would record over the stream's latest snapshot, nothing stored.
    #[tokio::test]
    async fn preview_collector_returns_a_fact_collectors_facts() {
        let (proj, svc, server) = boot().await;
        let ext = proj.path().join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nintent:\n  purpose: test\ncollectors:\n  - id: files\n    runtime: starlark\n    entry: files.star\n    needs: [tree.read]\n    facts: [work.files]\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("files.star"),
            "def transform(x):\n    return {\"facts\": [{\"measure\": \"work.files\", \"value\": len(files(\"**\")), \"subject\": ref.build(\"file\", \"src/a.rs\")}]}\n",
        )
        .unwrap();
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        let primary = svc.streams.list_streams().await.unwrap()[0].id;
        let snap = svc.snapshot_store.create_snapshot(primary).await.unwrap();
        svc.snapshot_store
            .capture_batch(vec![oxplow_db::FileSnapshot {
                id: 0,
                stream_id: primary,
                path: "src/a.rs".into(),
                blob_hash: Some(svc.blobs.write(b"fn a() {}\n").unwrap()),
                size_bytes: 10,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: None,
                content_hash: None,
            }])
            .await
            .unwrap();
        let r = server
            .preview_collector(
                rmcp::model::Extensions::new(),
                Parameters(PreviewCollectorParams {
                    owner: "work".into(),
                    id: "files".into(),
                    stream_id: None,
                    event: None,
                }),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(v["facts"][0]["measure"], "work.files", "{v}");
        assert_eq!(v["facts"][0]["value"], 1, "{v}");
        assert_eq!(v["facts"][0]["subject"], "file:src/a.rs", "{v}");
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
    }

    /// `read` reads a work item from the active list, whichever it is: the
    /// item while oxplow's tasks are the list, nothing with none.
    #[tokio::test]
    async fn a_work_item_reads_from_the_active_list() {
        let (_proj, services, server) = boot().await;
        let id = file_task(&services, None, "round trip").await;
        let read = |server: &OxplowMcp| {
            let server = server.clone();
            async move {
                server
                    .read(
                        rmcp::model::Extensions::new(),
                        Parameters(ReadInput {
                            reference: format!("work_item:oxplow:{id}"),
                            stream: None,
                        }),
                    )
                    .await
                    .map(|r| serde_json::from_str::<serde_json::Value>(&text_payload(r)).unwrap())
            }
        };
        assert_eq!(read(&server).await.unwrap()["row"]["title"], "round trip");
        services
            .config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), "none".into());
        let config = oxplow_app::config_service::read_config(&services.config);
        services
            .capabilities
            .publish(&config, &services.db)
            .await
            .unwrap();
        let gone = read(&server).await.unwrap_err();
        assert!(gone.message.contains("nothing at"), "{gone:?}");
    }

    /// A request's parts as the control plane passes them on: the
    /// session's principal in its extensions, plus whatever `headers` it
    /// carried (which name nobody).
    fn parts_as(
        thread: oxplow_domain::ThreadId,
        stream: oxplow_domain::StreamId,
        headers: &[(&str, &str)],
    ) -> http::request::Parts {
        let mut b = http::Request::builder().uri("http://h/mcp");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut parts = b.body(()).unwrap().into_parts().0;
        parts.extensions.insert(Principal {
            session: oxplow_domain::AgentSessionId::new(1),
            thread,
            stream,
            harness: "claude".into(),
        });
        parts
    }

    /// The identity of the primary stream's writer thread — what oxplow's
    /// harness configs send, and what a status-changing tool needs.
    async fn as_writer(services: &oxplow_app::Services) -> rmcp::model::Extensions {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        extensions_for(parts_as(thread.id, stream.id, &[]))
    }

    fn extensions_for(parts: http::request::Parts) -> rmcp::model::Extensions {
        let mut ext = rmcp::model::Extensions::new();
        ext.insert(parts);
        ext
    }

    #[test]
    fn mcp_caller_is_the_bearers_session_whatever_the_request_says() {
        let c = McpCaller::from_parts(&parts_as(
            oxplow_domain::ThreadId::new(3),
            oxplow_domain::StreamId::new(2),
            &[("x-oxplow-thread", "thr9"), ("x-oxplow-stream", "str9")],
        ));
        assert_eq!(c.session_id, Some(oxplow_domain::AgentSessionId::new(1)));
        assert_eq!(c.thread_id, Some(oxplow_domain::ThreadId::new(3)));
        assert_eq!(c.stream_id, Some(oxplow_domain::StreamId::new(2)));
        assert_eq!(c.actor().source(), "agent:thr3");
        // Headers or a query alone, with no principal, are nobody.
        let bare = http::Request::builder()
            .uri("http://h/mcp?thread=thr5&stream=str1")
            .header("x-oxplow-thread", "thr5")
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert_eq!(McpCaller::from_parts(&bare), McpCaller::default());
        assert_eq!(
            caller_of(&rmcp::model::Extensions::new()),
            McpCaller::default()
        );
    }

    #[tokio::test]
    async fn run_command_refuses_an_anonymous_connection_and_lists_for_agents() {
        let (_proj, _services, server) = boot().await;
        let err = server
            .run_command_as(
                &McpCaller::default(),
                RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({"key": "zones", "value": []}),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        // P4.8: recording a metric is a command like any write — refused
        // without an identity, and no fact lands.
        let err = server
            .run_command_as(
                &McpCaller::default(),
                RunCommandParams {
                    id: "oxplow.metric.record".into(),
                    input: serde_json::json!({"key": "oxplow.rust.unsafe_blocks", "value": 1.0}),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        // Listing needs no identity and shows agent-invocable commands.
        let specs = listed(&server, ListCommandsParams::default())
            .await
            .unwrap();
        let names: Vec<&str> = specs.iter().map(|s| s["id"].as_str().unwrap()).collect();
        for expected in [
            "oxplow.config.set",
            "oxplow.config.list_keys",
            "oxplow.work_item.transition",
            "oxplow.metric.record",
            "oxplow.collector.sync",
            "oxplow.metric.rebuild",
            "oxplow.metric.scaffold",
        ] {
            assert!(names.contains(&expected), "{names:?}");
        }
        assert!(specs.iter().all(|s| s["summary"].is_string()));
    }

    /// `list_commands` with no arguments: every command's id and summary,
    /// nothing more.
    async fn listed(
        server: &OxplowMcp,
        params: ListCommandsParams,
    ) -> Result<Vec<serde_json::Value>, McpError> {
        let result = server
            .list_commands(rmcp::model::Extensions::new(), Parameters(params))
            .await?;
        let text = result.content[0].as_text().unwrap().text.clone();
        Ok(serde_json::from_str(&text).unwrap())
    }

    /// An agent discovers a command in two small steps instead of one huge
    /// one: the index (ids and summaries), then one command — or one
    /// namespace — with its schema, its example input, whether a person
    /// confirms it and whether it undoes. The person-facing parts of a
    /// spec (`ui`, `op`, `atomicity`, …) never reach it.
    #[tokio::test]
    async fn list_commands_is_an_index_then_one_command_with_its_example() {
        let (_proj, _services, server) = boot().await;
        let index = listed(&server, ListCommandsParams::default())
            .await
            .unwrap();
        for entry in &index {
            let mut keys: Vec<&String> = entry.as_object().unwrap().keys().collect();
            keys.sort();
            assert_eq!(keys, ["id", "summary"], "{entry}");
        }
        let size = serde_json::to_string(&index).unwrap().len();
        // Fetched on demand, not every turn: ~4K tokens (every full spec was 164 KB).
        assert!(size < 17_000, "the index is {size} bytes");
        // A summary is cut at its first sentence; the command shows it whole.
        assert_eq!(
            first_sentence("Open a slot (`thread:thr1`). It starts nothing, e.g. no process."),
            "Open a slot (`thread:thr1`)."
        );
        assert_eq!(first_sentence("One sentence"), "One sentence");

        let one = listed(
            &server,
            ListCommandsParams {
                id: Some("oxplow.config.get".into()),
                prefix: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(one.len(), 1);
        let mut keys: Vec<&String> = one[0].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "confirm",
                "example",
                "id",
                "input_schema",
                "summary",
                "undoable"
            ]
        );
        assert!(one[0]["example"].is_object(), "{}", one[0]);
        // The example is shown once, not again inside the schema.
        assert!(one[0]["input_schema"].get("examples").is_none());

        let area = listed(
            &server,
            ListCommandsParams {
                id: None,
                prefix: Some("oxplow.config.".into()),
            },
        )
        .await
        .unwrap();
        let ids: Vec<&str> = area.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"oxplow.config.set"), "{ids:?}");
        assert!(
            ids.iter().all(|id| id.starts_with("oxplow.config.")),
            "{ids:?}"
        );
        assert!(area.iter().all(|s| s["input_schema"].is_object()));

        // An id it doesn't offer says so and names the ones that end alike.
        let err = listed(
            &server,
            ListCommandsParams {
                id: Some("config.get".into()),
                prefix: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("oxplow.config.get"), "{}", err.message);
    }

    /// The thread header is a claim, not a proof: an unknown thread, or a
    /// stream header that isn't the thread's stream, is refused, and
    /// `transition_tasks` needs an identity like `run_command` does.
    #[tokio::test]
    async fn commands_refuse_unknown_or_mismatched_callers() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot().await;
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let list = || RunCommandParams {
            id: "oxplow.config.list_keys".into(),
            input: serde_json::json!({}),
        };
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(oxplow_domain::ThreadId::new(999)),
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown thread"), "{err}");
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: Some(oxplow_domain::StreamId::new(999)),
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("belongs to"), "{err}");
        server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap();
        let err = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: None,
                    stream_id: None,
                },
                list(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
    }

    /// The zones write path after `set_zones` (tsk392): the agent runs
    /// `oxplow.config.set` through `run_command`, and the run is audited to the
    /// calling thread.
    #[tokio::test]
    async fn run_command_sets_zones_as_the_calling_thread() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (proj, services, server) = boot().await;
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let parts = parts_as(thread.id, stream.id, &[]);
        let out = server
            .run_command(
                extensions_for(parts),
                Parameters(RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({
                        "key": "zones",
                        "value": [{"match": "src/**", "zone": "core"}]
                    }),
                }),
            )
            .await
            .unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        let outcome: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(outcome["result"]["changed"], true);
        assert!(outcome["audit_id"].is_number());
        let yaml = std::fs::read_to_string(proj.path().join(".oxplow/project.yaml")).unwrap();
        assert!(yaml.contains("zone: core"), "{yaml}");
        assert_eq!(services.config.read().unwrap().zones.len(), 1);
        let events = services.event_log_store.read_after(0, 20).await.unwrap();
        let executed = events
            .iter()
            .find(|e| e.envelope.event_type == "command.executed")
            .expect("command.executed logged");
        assert_eq!(executed.envelope.source, format!("agent:{}", thread.id));
        assert_eq!(executed.envelope.anchors.thread_id, Some(thread.id));
        assert_eq!(executed.envelope.payload["command"], "oxplow.config.set");
        // A human-only key is not the agent's to set: the run is kept as a
        // proposal for a person, and the agent is told so — a success,
        // since the request is recorded and needs nothing more from it.
        let out = server
            .run_command_as(
                &McpCaller {
                    session_id: None,
                    thread_id: Some(thread.id),
                    stream_id: Some(stream.id),
                },
                RunCommandParams {
                    id: "oxplow.config.set".into(),
                    input: serde_json::json!({"key": "agentPromptAppend", "value": "be brief"}),
                },
            )
            .await
            .unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        let proposed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(proposed["kind"], "proposed");
        let proposal = proposed["proposal"].as_str().unwrap().to_string();
        assert!(proposal.starts_with("proposal:"), "{proposed}");
        let message = proposed["message"].as_str().unwrap();
        assert!(message.contains(&proposal), "{message}");
        assert!(message.contains("Alerts"), "{message}");
        assert!(message.contains("in this thread"), "{message}");
        assert!(message.contains("v_command_proposal"), "{message}");
        assert!(message.contains("don't run it again"), "{message}");
        let pending = services
            .commands
            .proposal_store()
            .list_pending()
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].dry_run.as_ref().unwrap()["after"], "be brief");
        let yaml = std::fs::read_to_string(proj.path().join(".oxplow/project.yaml")).unwrap();
        assert!(!yaml.contains("be brief"), "nothing written: {yaml}");
    }

    /// A thread on `stream`, made as a person through `oxplow.thread.create`.
    async fn new_thread(
        services: &oxplow_app::Services,
        stream: oxplow_domain::StreamId,
        title: &str,
    ) -> oxplow_domain::Thread {
        let out = services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                oxplow_app::commands::thread::CREATE,
                serde_json::json!({
                    "stream": oxplow_domain::refs::build::stream_ref(stream),
                    "title": title,
                }),
                false,
            )
            .await
            .unwrap();
        serde_json::from_value(out.result).unwrap()
    }

    /// `read` reads a file in the calling session's stream, not the
    /// primary's.
    #[tokio::test]
    async fn read_reads_files_in_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (proj, services, server) = boot().await;
        let primary = services.stream_store.list().await.unwrap().pop().unwrap();
        std::fs::write(proj.path().join("who.txt"), "primary").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("who.txt"), "mine").unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = services.stream_store.upsert(&other).await.unwrap();
        let thread = new_thread(&services, other_id, "t").await;
        let out = server
            .read(
                extensions_for(parts_as(thread.id, other_id, &[])),
                Parameters(ReadInput {
                    reference: "file:who.txt".into(),
                    stream: None,
                }),
            )
            .await
            .unwrap();
        let read: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(read["content"], "mine");
        // Naming a stream reads its worktree instead.
        let out = server
            .read(
                extensions_for(parts_as(thread.id, other_id, &[])),
                Parameters(ReadInput {
                    reference: "file:who.txt".into(),
                    stream: Some(oxplow_domain::refs::build::stream_ref(primary.id)),
                }),
            )
            .await
            .unwrap();
        let read: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(read["content"], "primary");
        let bad = server
            .read(
                extensions_for(parts_as(thread.id, other_id, &[])),
                Parameters(ReadInput {
                    reference: "file:who.txt".into(),
                    stream: Some("str1".into()),
                }),
            )
            .await
            .unwrap_err();
        assert!(bad.message.contains("stream:str2"), "{}", bad.message);
    }

    /// tsk574: the extension and lens tools without a `stream_id` read
    /// the calling thread's stream — an agent in a worktree sees the
    /// extension it just wrote there, not the primary's.
    #[tokio::test]
    async fn validate_extension_defaults_to_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (_proj, services, server) = boot().await;
        let primary = services.stream_store.list().await.unwrap().pop().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let ext = elsewhere.path().join("oxplow/extensions/mine");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: mine\nintent:\n  purpose: test\n",
        )
        .unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = services.stream_store.upsert(&other).await.unwrap();
        let thread = new_thread(&services, other_id, "t").await;
        let caller = || extensions_for(parts_as(thread.id, other_id, &[]));
        server
            .validate_extension(
                caller(),
                Parameters(ValidateExtensionParams {
                    name: "mine".into(),
                    stream_id: None,
                }),
            )
            .await
            .unwrap();
    }

    /// P6.C1: an agent shows an answer in its thread; the tool returns its
    /// ref and what the person sees, as text.
    #[tokio::test]
    async fn show_lens_records_an_answer_and_returns_its_text() {
        let (_proj, services, server) = boot().await;
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .show_lens(
                    as_writer(&services).await,
                    Parameters(ShowLensParams {
                        lens: None,
                        spec: Some(
                            serde_json::from_value(serde_json::json!({
                                "title": "Streams",
                                "query": "SELECT kind FROM v_stream",
                            }))
                            .unwrap(),
                        ),
                        params: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert!(
            out["answer"].as_str().unwrap().starts_with("answer:"),
            "{out}"
        );
        assert!(
            out["text"].as_str().unwrap().contains("| primary |"),
            "{out}"
        );
        // Without a thread identity there's nowhere to show it.
        let err = server
            .show_lens(
                rmcp::model::Extensions::new(),
                Parameters(ShowLensParams {
                    lens: Some("x/y".into()),
                    spec: None,
                    params: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(!err.message.is_empty());
    }

    /// Helper to write a wiki body to disk.
    async fn seed_wiki(project: &std::path::Path, slug: &str, body: &str) {
        let wiki_dir = project.join(".oxplow").join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(wiki_dir.join(format!("{slug}.md")), body).unwrap();
    }

    #[tokio::test]
    async fn wiki_ref_drift_reports_status_per_ref() {
        let (proj, svc, server) = boot().await;
        seed_wiki(proj.path(), "intro", "see [[crates/foo.rs]]").await;
        oxplow_app::wiki_pages::sync_page(&svc.db, &svc.vocabulary, proj.path(), "intro")
            .await
            .unwrap();
        // The file IS referenced but has no pin (no snapshot service in tests).
        let r = server
            .wiki_ref_drift(
                rmcp::model::Extensions::new(),
                Parameters(WikiRefDriftParams {
                    slug: "intro".into(),
                    path: "crates/foo.rs".into(),
                }),
            )
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("\"status\""), "{body}");
        assert!(body.contains("no_pin"), "expected no_pin: {body}");
        // A path the page doesn't reference at all.
        let r2 = server
            .wiki_ref_drift(
                rmcp::model::Extensions::new(),
                Parameters(WikiRefDriftParams {
                    slug: "intro".into(),
                    path: "crates/zzz.rs".into(),
                }),
            )
            .await
            .unwrap();
        assert!(text_payload(r2).contains("not_a_ref"));
    }

    // ---- Pure helpers: parse_status / parse_priority ----

    /// Point the in-memory services' `role` at a mock provider at `base`.
    fn assign_mock_role(services: &Services, base: String, role: &str) {
        use oxplow_app::ai_service::{ProviderConfig, Role, RoleBinding};
        use oxplow_app::exec_consent::{approve_program, list, ProgramKind};
        // The shipped providers are scripts a person approves first.
        let project = services.layout.project_dir.clone();
        let config = services.config.read().unwrap().clone();
        let extensions = services.extension_catalog.get(&project);
        for p in list(&services.approvals, &project, &config, &extensions) {
            if p.kind == ProgramKind::AiProvider {
                approve_program(
                    &services.approvals,
                    &project,
                    &config,
                    &extensions,
                    p.kind,
                    &p.name,
                    p.version.as_deref().unwrap(),
                )
                .unwrap();
            }
        }
        services
            .ai
            .save_provider(
                ProviderConfig {
                    id: "mock".into(),
                    kind: "openai_compatible".into(),
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        let role: Role = serde_json::from_value(serde_json::json!(role)).unwrap();
        services
            .ai
            .set_role(&role, Some(RoleBinding::new("mock", "m")))
            .unwrap();
    }

    fn risky() -> std::collections::BTreeMap<String, AiQuestionParam> {
        std::collections::BTreeMap::from([(
            "risky".to_string(),
            AiQuestionParam {
                kind: "choice".into(),
                instructions: "Is it risky?".into(),
                options: Some(vec!["yes".into(), "no".into()]),
                levels: None,
            },
        )])
    }

    /// The model calls an agent asks for are paid with the person's key:
    /// each is a recorded computation, recorded as the asking thread — the
    /// same question twice is one call — on its tool's own role, with its
    /// text capped; a connection with no thread identity asks nothing.
    #[tokio::test]
    async fn ai_tools_decide_and_summarize() {
        let (_proj, services, server) = boot().await;
        let me = as_writer(&services).await;
        let thread = oxplow_domain::refs::build::thread_ref(caller_of(&me).thread_id.unwrap());
        // Unassigned role: a clear error, not a crash.
        let err = server
            .ai_summarize(
                me.clone(),
                Parameters(AiSummarizeParams {
                    text: "t".into(),
                    focus: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("summarize"), "{}", err.message);

        let answer = serde_json::json!({"answers": {"risky": {"type": "choice", "choice": "yes", "probabilities": {"yes": 0.7, "no": 0.3}}}});
        let (base, seen) = oxplow_ai_fake::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": answer.to_string()}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        assign_mock_role(&services, base.clone(), "decide");
        for cached in [false, true] {
            let out: serde_json::Value = serde_json::from_str(&text_payload(
                server
                    .ai_decide(
                        me.clone(),
                        Parameters(AiDecideParams {
                            state: "a diff".into(),
                            questions: risky(),
                        }),
                    )
                    .await
                    .unwrap(),
            ))
            .unwrap();
            assert_eq!(out["answers"]["risky"]["choice"], "yes");
            assert_eq!(out["cached"], cached);
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("Is it risky?"));

        let err = server
            .ai_decide(
                me.clone(),
                Parameters(AiDecideParams {
                    state: "s".into(),
                    questions: std::collections::BTreeMap::from([(
                        "x".to_string(),
                        AiQuestionParam {
                            kind: "maybe".into(),
                            instructions: "?".into(),
                            options: None,
                            levels: None,
                        },
                    )]),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("maybe"), "{}", err.message);
        let err = server
            .ai_decide(
                me.clone(),
                Parameters(AiDecideParams {
                    state: "x".repeat(AI_TEXT_MAX + 1),
                    questions: risky(),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("characters"), "{}", err.message);
        let err = server
            .ai_decide(
                rmcp::model::Extensions::new(),
                Parameters(AiDecideParams {
                    state: "a diff".into(),
                    questions: risky(),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("thread identity"), "{}", err.message);

        let (base, seen) = oxplow_ai_fake::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": " short "}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        assign_mock_role(&services, base, "summarize");
        // A recorded computation (tsk571): asking twice calls once.
        for _ in 0..2 {
            let out = text_payload(
                server
                    .ai_summarize(
                        me.clone(),
                        Parameters(AiSummarizeParams {
                            text: "long".into(),
                            focus: Some("risks".into()),
                        }),
                    )
                    .await
                    .unwrap(),
            );
            assert_eq!(out, "short");
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("risks"));
        let callers = services
            .sql
            .query_sql("SELECT DISTINCT caller FROM v_ai_call", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&callers.rows).unwrap(),
            serde_json::json!([[thread]])
        );
    }

    /// A file's symbols come from its language server when one runs; with
    /// none, the tool reads the file's live content with oxplow's own
    /// tree-sitter analysis and says that's what answered.
    #[tokio::test]
    async fn code_symbols_fall_back_to_tree_sitter_and_say_so() {
        let (proj, services, server) = boot().await;
        std::fs::create_dir_all(proj.path().join("src")).unwrap();
        std::fs::write(
            proj.path().join("src/x.rs"),
            "fn alpha() {}\n\nstruct Beta;\n",
        )
        .unwrap();
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .code_symbols(
                    rmcp::model::Extensions::new(),
                    Parameters(CodeSymbolsParams {
                        stream_id: Some(stream_id),
                        path: "src/x.rs".into(),
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["source"], "tree_sitter");
        let names: Vec<_> = out["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"alpha".to_string()), "{out}");
        assert_eq!(out["symbols"][0]["startLine"], 1);
        // The caller's own stream when none is named.
        let again = server
            .code_symbols(
                rmcp::model::Extensions::new(),
                Parameters(CodeSymbolsParams {
                    stream_id: None,
                    path: "src/x.rs".into(),
                }),
            )
            .await;
        assert!(again.is_ok(), "{again:?}");
    }

    /// A workspace query needs a running language server; without one the
    /// error says where the stored symbols are instead.
    #[tokio::test]
    async fn code_workspace_symbols_need_a_language_server_and_point_at_v_symbol() {
        let (_proj, services, server) = boot().await;
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let err = server
            .code_workspace_symbols(
                rmcp::model::Extensions::new(),
                Parameters(CodeWorkspaceSymbolsParams {
                    stream_id: Some(stream_id),
                    language: "rust".into(),
                    query: "alpha".into(),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("v_symbol"), "{}", err.message);
    }

    /// Blame reads through the Vcs provider: each line of the working copy
    /// names the commit that last changed it, an uncommitted line none.
    #[tokio::test]
    async fn vcs_blame_names_the_commit_of_each_line() {
        let (proj, _services, server) = boot().await;
        std::fs::write(proj.path().join("b.txt"), "one\ntwo\n").unwrap();
        let repo = git2::Repository::open(proj.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("b.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "add b", &tree, &[&parent])
            .unwrap();
        std::fs::write(proj.path().join("b.txt"), "one\ntwo\nthree\n").unwrap();
        let lines: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .vcs_blame(
                    rmcp::model::Extensions::new(),
                    Parameters(BlameParams {
                        stream_id: None,
                        path: "b.txt".into(),
                        revision: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        let lines = lines.as_array().unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].to_string().contains(&commit.to_string()),
            "{}",
            lines[0]
        );
        assert!(
            !lines[2].to_string().contains(&commit.to_string()),
            "{}",
            lines[2]
        );
    }

    #[tokio::test]
    async fn ensure_change_analyzes_a_commit_for_agents() {
        let (proj, _services, server) = boot().await;
        std::fs::write(proj.path().join("a.rs"), "fn a() {}\n").unwrap();
        let repo = git2::Repository::open(proj.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.rs")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "a", &tree, &[&parent])
            .unwrap();

        let out: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .ensure_change(
                    rmcp::model::Extensions::new(),
                    Parameters(EnsureChangeParams {
                        kind: "commit".into(),
                        sha: Some("HEAD".into()),
                        effort_id: None,
                        stream_id: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["status"], "done");
        let err = server
            .ensure_change(
                rmcp::model::Extensions::new(),
                Parameters(EnsureChangeParams {
                    kind: "sideways".into(),
                    sha: None,
                    effort_id: None,
                    stream_id: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("sideways"), "{}", err.message);
    }
}
