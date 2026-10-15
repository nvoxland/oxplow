//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Tools are thin handlers that
//! delegate into `oxplow-app` services — we never duplicate business
//! logic between the Tauri command surface and the MCP tool surface.
//!
//! Each tool takes a single `Parameters<T>` argument (rmcp
//! convention); request shapes are defined as `serde + JsonSchema`
//! structs alongside the tool methods.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::*;
use rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::ref_resolver::{self, RefSummary};
use oxplow_app::session_auth::Principal;
use oxplow_app::Services;
use oxplow_domain::comment::CommentThread;
use oxplow_domain::stores::CommentStore;
use oxplow_domain::{CommentStatus, StreamId, ThreadId};

mod lenient_params;
// Drop-in for rmcp's `Parameters` that tolerates camelCase/kebab aliases
// of our snake_case param fields, so weak models stop tripping on
// `-32602 missing field`. See `lenient_params` for the rationale.
use lenient_params::Parameters;

/// A comment thread plus the typed context it was anchored in, resolved
/// for the agent. `primary` is the comment's target (the nearest region
/// the selection sat in); `context_chain` is the ancestor regions
/// (innermost→outermost); `referenced` are the canonical refs found
/// inside the selection itself. Returned by `list_comments`.
#[derive(Debug, Serialize)]
struct EnrichedCommentThread {
    thread: CommentThread,
    primary: RefSummary,
    context_chain: Vec<RefSummary>,
    referenced: Vec<RefSummary>,
}

#[derive(Clone)]
pub struct OxplowMcp {
    services: Arc<Services>,
    tool_router: ToolRouter<Self>,
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReadParams {
    /// What to read, by its ref: `file:src/a.rs@git:HEAD#L10-40`,
    /// `work_item:oxplow:tsk12`, `commit:<sha>`, `event:<id>#output`,
    /// `lens:<ext>/<slug>`.
    #[serde(rename = "ref")]
    pub reference: String,
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
pub struct ThreadIdParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkItemRefParams {
    /// The item's ref, `work_item:<provider>:<id>`.
    #[serde(rename = "ref")]
    pub item_ref: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListCommentsParams {
    /// `"thread"` (id = `thr…`) or `"stream"` (id = `str…`). Optional —
    /// when omitted it's inferred from `id`'s prefix.
    pub scope: Option<String>,
    /// The thread (`thr…`) or stream (`str…`) id whose comments to list.
    pub id: Option<String>,
    /// Filter: `"all"` (default), `"open"`, or `"needs_response"`.
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WikiRefDriftParams {
    /// Wiki slug (without `.md`).
    pub slug: String,
    /// Repo-relative file path referenced by the page.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PageRefParams {
    /// Ref kind: `wiki`, `work_item`, `file`, `dir`, `commit`,
    /// `finding`, `thread_note` (see .context/refs.md).
    pub kind: String,
    /// The ref's id within the kind: a repo-relative path for files and
    /// dirs, `oxplow:tsk42` for a task, the sha for a commit, the slug
    /// for a wiki page.
    pub id: String,
    #[serde(default = "default_page_ref_limit")]
    pub limit: u32,
}

fn default_page_ref_limit() -> u32 {
    100
}

/// A place in a stream's file (`code_*` tools). Line and column are
/// 1-based, like editors and `v_diagnostic`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodePositionParams {
    pub stream_id: String,
    /// Workspace-relative path (`src/lib.rs`).
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub col: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeReferencesParams {
    pub stream_id: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// Include the declaration itself (default true).
    #[serde(default)]
    pub include_declaration: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeFileParams {
    pub stream_id: String,
    /// Workspace-relative path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeWorkspaceSymbolsParams {
    pub stream_id: String,
    /// The language server to ask (`rust`, `typescript`, `python`, …).
    pub language: String,
    /// Symbol name query (fuzzy, server-defined); empty lists what the
    /// server returns.
    pub query: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeCallHierarchyParams {
    pub stream_id: String,
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// `incoming` (who calls the symbol) or `outgoing` (what it calls).
    pub direction: oxplow_domain::code_intel::CallDirection,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListCodeUnitsParams {
    pub stream_id: String,
    /// Repo-relative path of the file to list units for.
    pub path: String,
}

/// Optional stream selector shared by the stream-scoped VCS read tools.
/// Omit `stream_id` to read the caller's own stream (the primary only for
/// a caller with no thread identity).
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct GitStreamParams {
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DiffParams {
    /// The stream whose workspace to diff; the caller's when absent.
    pub stream_id: Option<String>,
    /// The older side: `working`, `snap:<id>` or `git:<rev>`; absent =
    /// the empty tree (everything added).
    pub from: Option<String>,
    /// The newer side (e.g. `working`).
    pub to: String,
    /// Compare `to` against where it forked from `from` rather than
    /// against `from` itself — a branch's own changes against `git:main`.
    #[serde(default)]
    pub since_fork: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BlameParams {
    /// The stream whose workspace to blame in; the caller's when absent.
    pub stream_id: Option<String>,
    /// Workspace-relative file path.
    pub path: String,
    /// Which version: `working` (default; uncommitted lines name no
    /// revision) or `git:<rev>`.
    pub revision: Option<String>,
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
pub struct ListExtensionsParams {
    /// Stream whose worktree to read `oxplow/extensions/` from. Omit for
    /// your own stream (the calling thread's; the primary when the call
    /// carries no thread).
    pub stream_id: Option<String>,
    /// One extension's name: its whole listing (every lens, collector,
    /// effect, …) instead of the summary of each.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ShowLensParams {
    /// An existing lens to show (`<extension>/<slug>`, see `list_lenses`).
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
    /// Lens id: `<extension>/<slug>` (see `list_lenses`).
    pub id: String,
    /// Stream whose worktree to read from; omit for your own.
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunLensParams {
    /// Lens id: `<extension>/<slug>`.
    pub id: String,
    /// Param overrides by name (see the lens's `params`); the rest use
    /// their defaults. Unknown names are rejected.
    pub params: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Stream whose worktree to read from; omit for your own.
    pub stream_id: Option<String>,
    /// Your thread (`thr…`): bound into a lens's `thread_id` param (and its
    /// stream into `stream_id`) unless `params` sets them. Omit to use the
    /// stream's selected thread — what the person sees.
    pub thread_id: Option<String>,
    /// `text` (default): the lens's text rendering (what it shows — a
    /// chart's series, a grid's children) with the run's columns, row
    /// count, params and alert, but not the rows. `json`: the full run,
    /// rows included, for when you need to parse the values.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunLensActionParams {
    /// Lens id: `<extension>/<slug>`.
    pub id: String,
    /// The action's id, as the lens declares it (see `read` of `lens:<ext>/<slug>` → `actions`).
    pub action: String,
    /// Param overrides by name, as for `run_lens`.
    pub params: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Stream whose worktree to read the lens from; omit for your own.
    pub stream_id: Option<String>,
    /// Your thread, as for `run_lens`.
    pub thread_id: Option<String>,
    /// For a row action (`row: true` in `read` of `lens:<ext>/<slug>` → `actions`): the row
    /// it runs on, column → value, as `run_lens` returned it.
    pub row: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EnsureChangeParams {
    /// `commit` (a commit vs its parent), `effort` (an effort's start →
    /// end), or `working` (a stream's uncommitted work vs HEAD).
    pub kind: String,
    /// For `commit`: a sha or revspec (`HEAD`, `main~2`).
    pub sha: Option<String>,
    /// For `effort`: the effort id (`eff42`).
    pub effort_id: Option<String>,
    /// For `working` and `commit`: the stream id; your own when omitted.
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
pub struct RunCollectorParams {
    /// The collector's ref: `collector:<extension>/<id>` (the extension's
    /// folder under `oxplow/extensions/`, the `id` in its `collectors:`).
    pub collector: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PreviewCollectorParams {
    /// The extension that declares it (its folder under `oxplow/extensions/`).
    pub owner: String,
    /// The collector's `id` in its `collectors:`.
    pub id: String,
    /// Stream whose worktree to run it from; omit for your own.
    pub stream_id: Option<String>,
    /// The event an `on:` collector reacts to, as a fixture writes it:
    /// `{ type, v?, payload?, subject?, anchors? }`. The script gets it as
    /// the event envelope, carried to the type's newest version.
    pub event: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReviewExtensionParams {
    /// Git URL to review for an install (a repo whose root holds
    /// `extension.yaml`). Pass this or `name`.
    pub git_url: Option<String>,
    /// Branch, tag or commit; omit for the default branch.
    pub git_ref: Option<String>,
    /// An installed extension's name, to review its update. Pass this or
    /// `git_url`.
    pub name: Option<String>,
    /// Stream whose worktree it goes into; omit for your own.
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ValidateExtensionParams {
    /// Extension folder name under `oxplow/extensions/`.
    pub name: String,
    /// Stream whose worktree to read from; omit for your own.
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct QuerySqlParams {
    /// One read-only `SELECT` or `WITH` statement over the published
    /// models (`v_model` lists them, `v_model_column` documents their
    /// columns). Use `?1`, `?2`, … for parameters.
    pub sql: String,
    /// Positional parameter values for `?1`, `?2`, ….
    pub params: Option<Vec<serde_json::Value>>,
    /// Row cap (default 500, max 10000). `truncated: true` in the result
    /// means more rows existed.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SiteSearchParams {
    /// Free text. Tokens are matched as prefixes (stemmed); ranking is BM25.
    pub query: String,
    /// Scope file/stream-bound hits to one worktree (project-global hits like
    /// wiki are always included). Omit to search every stream.
    pub stream_id: Option<String>,
    /// Restrict to a subset of kinds: `task | comment | note | wiki | file`.
    pub kinds: Option<Vec<String>>,
    /// Max hits to return (default 50).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSkillParams {
    /// Skill name, e.g. `oxplow-extension`.
    pub name: String,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Read one of oxplow's skills (its SKILL.md): how to do a kind of oxplow \
                       work, e.g. `oxplow-extension` for building lenses. For agents that don't \
                       load skill files themselves; your instructions list the names."
    )]
    async fn get_skill(
        &self,
        params: Parameters<GetSkillParams>,
    ) -> Result<CallToolResult, McpError> {
        let name = params.0.name;
        // What's offered now: core's and the active implementations'.
        let text = oxplow_app::capabilities::agent_text(&self.services);
        match text.skill_body(&name) {
            Some(body) => Ok(CallToolResult::success(vec![ContentBlock::text(
                body.to_string(),
            )])),
            None => {
                let names: Vec<&str> = text.skill_index().into_iter().map(|(n, _)| n).collect();
                Err(McpError::invalid_params(
                    format!("no skill `{name}` (skills: {})", names.join(", ")),
                    None,
                ))
            }
        }
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
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let tid = &params.0.thread_id;
        expect_id_kind("get_open_page", "thread_id", tid, ID_THREAD)?;
        let thread = oxplow_domain::ThreadId::try_from_str(tid)
            .ok_or_else(|| McpError::invalid_params("expected a thread id (thr…)", None))?;
        let Some(page) = self.services.thread_runtime.open_page(&thread) else {
            return json_result(&serde_json::json!({ "open": null }));
        };
        let detail: serde_json::Value = page
            .detail_json
            .as_deref()
            .and_then(|d| serde_json::from_str(d).ok())
            .unwrap_or(serde_json::Value::Null);
        let mut lens_text = serde_json::Value::Null;
        if page.kind == "lens" {
            if let Ok(lens_id) =
                oxplow_domain::refs::build::id_of(&page.page_id, oxplow_domain::refs::kind::LENS)
            {
                // The app shows the main worktree's lenses, whatever the
                // thread's stream.
                let root = self.services.worktrees.project_dir();
                let lens_params = detail
                    .get("params")
                    .and_then(|p| p.as_object())
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                            .collect()
                    })
                    .unwrap_or_default();
                let ctx =
                    oxplow_app::extensions::lens_context(&self.services, None, Some(thread)).await;
                let read = async {
                    let run = oxplow_app::extensions::run_lens(
                        &self.services.sql,
                        &self.services.extension_catalog,
                        root,
                        &lens_id,
                        lens_params,
                        &ctx,
                    )
                    .await?;
                    oxplow_app::lens_text::text_run(&self.services, root, &run, &ctx).await
                };
                lens_text = match read.await {
                    Ok(text) => serde_json::to_value(text).map_err(internal)?,
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                };
            }
        }
        json_result(&serde_json::json!({
            "open": {
                "pageId": page.page_id,
                "kind": page.kind,
                "detail": detail,
                "reportedAt": page.reported_at,
            },
            "lens": lens_text,
        }))
    }

    #[tool(
        description = "List extension-declared data sources (code that pulls external records \
                       like GitHub PRs into the semantic layer): each source's spec (entities, \
                       schedule, env), its last run (status, row counts, error) and whether a \
                       person on this machine has approved its current script."
    )]
    async fn list_collectors(&self) -> Result<CallToolResult, McpError> {
        let root = self
            .services
            .worktrees
            .resolve(None)
            .await
            .into_local_path();
        let list = oxplow_app::collector_runner::list_collectors(
            &oxplow_app::collector_runner::Collectors::of(&self.services, &root),
        )
        .await
        .map_err(internal)?;
        json_result(&list)
    }

    #[tool(
        description = "Run an approved extension collector now (`collector`: its ref, `collector:<extension>/<id>`), refreshing its entities \
                       (`v_<extension>_<entity>`). You cannot approve a collector: running code \
                       the human hasn't approved fails, so ask them to use Settings → Data → \
                       Approve & Run. Returns row counts per entity."
    )]
    async fn run_collector(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<RunCollectorParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        // The `oxplow.collector.sync` command, as the caller: it never approves.
        let actor = self.verified_actor(&caller_of(&extensions)).await?;
        let out = self
            .services
            .commands
            .run(
                &actor,
                oxplow_app::collector_runner::SYNC,
                serde_json::json!({ "collector": p.collector }),
                false,
            )
            .await
            .map_err(command_error)?;
        json_result(&out.result)
    }

    #[tool(
        description = "Dry-run a collector from a stream's worktree and return what it would \
                       store, storing nothing: an entity collector's rows (first 50 per entity, \
                       coerced to the declared columns), or a fact collector's (`facts:`) facts \
                       as `{ owner, id, facts: [{ measure, value, subject, … }] }`, computed over \
                       the stream's latest snapshot (a collector that reads `input` rows is \
                       previewed through an example: `oxplow extension test`). Use it to check a collector you're writing, especially in a \
                       worktree stream: run_collector always runs the primary's copy, and \
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
        let p = params.0;
        check_optional_stream("preview_collector", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let collectors = oxplow_app::collector_runner::Collectors::of(&self.services, &root);
        // A fact collector (`facts:`) previews through the fact engine: the
        // facts it would record, over that stream's latest snapshot.
        if let Ok((_, spec)) =
            oxplow_app::collector_runner::find_collector(&collectors, &p.owner, &p.id)
        {
            if !spec.facts.is_empty() {
                let stream_id = stream
                    .as_deref()
                    .map(parse_stream_id)
                    .transpose()?
                    .map(|s| s.value());
                let facts = self
                    .services
                    .metrics
                    .preview_fact_collector_in(&p.owner, &spec, &root, stream_id, p.event.as_ref())
                    .await
                    .map_err(|m| McpError::invalid_params(m, None))?;
                return json_result(&serde_json::json!({
                    "owner": p.owner,
                    "id": p.id,
                    "facts": facts,
                }));
            }
        }
        let preview = oxplow_app::collector_runner::preview_collector(
            &collectors,
            &p.owner,
            &p.id,
            p.event.as_ref(),
            None,
        )
        .await
        .map_err(|e| match e {
            oxplow_app::collector_runner::RunCollectorError::NotFound => McpError::invalid_params(
                format!(
                    "no collector `{}/{}` in that stream's worktree (see list_extensions)",
                    p.owner, p.id
                ),
                None,
            ),
            oxplow_app::collector_runner::RunCollectorError::NeedsApproval(m)
            | oxplow_app::collector_runner::RunCollectorError::Failed(m)
            | oxplow_app::collector_runner::RunCollectorError::Disabled(m) => {
                McpError::invalid_params(m, None)
            }
            oxplow_app::collector_runner::RunCollectorError::Storage(e) => internal(e),
        })?;
        json_result(&preview)
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
        use oxplow_app::change_analysis::ChangeTarget;
        let p = params.0;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let need = |v: Option<String>, name: &str| {
            v.ok_or_else(|| {
                McpError::invalid_params(
                    format!("`{name}` is required for kind `{}`", p.kind),
                    None,
                )
            })
        };
        let target = match p.kind.as_str() {
            "commit" => ChangeTarget::Commit {
                sha: need(p.sha.clone(), "sha")?,
                stream_id: stream.clone(),
            },
            "effort" => ChangeTarget::Effort {
                effort_id: need(p.effort_id.clone(), "effort_id")?,
            },
            "working" => ChangeTarget::Working {
                stream_id: need(stream.clone(), "stream_id")?,
            },
            other => {
                return Err(McpError::invalid_params(
                    format!("kind `{other}` isn't commit, effort or working"),
                    None,
                ))
            }
        };
        let row = oxplow_app::change_analysis::ensure_change(&self.services, target)
            .await
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        json_result(&row)
    }

    #[tool(
        description = "List oxplow's AI roles — the built-ins (main, summarize, decide), then \
                       each active extension's AI profiles (`<extension>/<name>`, with the role \
                       it falls back to): which provider and model each is bound to and would \
                       use now, or none, plus the configured providers and whether each has a \
                       key. Keys are never shown. The person manages these in Settings → AI."
    )]
    async fn list_ai_roles(&self) -> Result<CallToolResult, McpError> {
        json_result(&self.services.ai.settings().await.map_err(ai_error)?)
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
        let p = params.0;
        check_optional_stream("review_extension", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let layer = self.services.sql.clone();
        let commands = self.services.commands.as_ref();
        let review = match (p.git_url.as_deref(), p.name.as_deref()) {
            (Some(url), None) => {
                oxplow_app::extensions::review_extension(
                    &layer,
                    &self.services.extension_catalog,
                    &root,
                    url,
                    p.git_ref.as_deref(),
                    None,
                    commands,
                )
                .await
            }
            (None, Some(name)) => {
                oxplow_app::extensions::review_update(
                    &layer,
                    &self.services.extension_catalog,
                    &root,
                    name,
                    commands,
                )
                .await
            }
            _ => {
                return Err(McpError::invalid_params(
                    "pass either git_url (install) or name (update)",
                    None,
                ))
            }
        }
        .map_err(extension_error)?;
        json_result(&review)
    }

    #[tool(
        description = "List the project's extensions (folders under `oxplow/extensions/` in a \
                       stream's worktree): each one's name, whether it's enabled, any load \
                       errors and warnings, and how many lenses, collectors, effects, … it \
                       contributes. Pass `name` for one extension's whole listing. A lens is a \
                       saved query over the semantic layer plus how to show it; the human sees \
                       each one as a page (`list_lenses` lists them all). To build one, write \
                       `oxplow/extensions/<name>/extension.yaml` and `lenses/<slug>.yaml` with \
                       your normal file tools (see the oxplow-extension skill), then call \
                       `validate_extension`."
    )]
    async fn list_extensions(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ListExtensionsParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("list_extensions", params.0.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), params.0.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let listed = self.services.listed_extensions(&root).await;
        match &params.0.name {
            Some(name) => {
                let one: Vec<_> = listed.into_iter().filter(|e| &e.name == name).collect();
                if one.is_empty() {
                    return Err(McpError::invalid_params(
                        format!("no extension named `{name}`"),
                        None,
                    ));
                }
                json_result(&one)
            }
            None => json_result(&listed.iter().map(extension_summary).collect::<Vec<_>>()),
        }
    }

    #[tool(
        description = "List every lens across the project's extensions (id `<extension>/<slug>`, \
                       title, description, viz, params). Use `run_lens` to see exactly the rows \
                       the human sees on that lens's page."
    )]
    async fn list_lenses(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<StreamScopeParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("list_lenses", params.0.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), params.0.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let lenses: Vec<oxplow_app::extensions::Lens> = self
            .services
            .extension_catalog
            .get(&root)
            .iter()
            .flat_map(|e| e.lenses.clone())
            .collect();
        json_result(&lenses)
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
        params: Parameters<RunLensParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        check_optional_stream("run_lens", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let overrides = p
            .params
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
            .collect();
        let ctx = self
            .lens_context(stream.as_deref(), p.thread_id.as_deref())
            .await?;
        // Through its extension's models as that stream has them, compiled
        // and published nowhere: a lens over a model the author hasn't
        // published yet answers.
        let extension = p.id.split_once('/').map_or(p.id.as_str(), |(ext, _)| ext);
        let overlay = oxplow_app::extensions::lens_overlay(
            &self.services.sql,
            &self.services.extension_catalog,
            &root,
            extension,
        )
        .await;
        let run = oxplow_app::extensions::run_lens(
            &self.services.sql.with_overlay(overlay),
            &self.services.extension_catalog,
            &root,
            &p.id,
            overrides,
            &ctx,
        )
        .await
        .map_err(|e| lens_error(&p.id, e))?;
        match p.format.as_deref().unwrap_or("text") {
            "json" => json_result(&run),
            "text" => json_result(
                &oxplow_app::lens_text::text_run(&self.services, &root, &run, &ctx)
                    .await
                    .map_err(|e| lens_error(&p.id, e))?,
            ),
            other => Err(McpError::invalid_params(
                format!("format `{other}` isn't text or json"),
                None,
            )),
        }
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
        params: Parameters<RunLensActionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        check_optional_stream("run_lens_action", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let overrides = p
            .params
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
            .collect();
        let ctx = self
            .lens_context(stream.as_deref(), p.thread_id.as_deref())
            .await?;
        let on_behalf_of = self.verified_actor(&caller_of(&extensions)).await?;
        let row = p.row.map(|r| {
            r.into_iter()
                .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
                .collect()
        });
        let out = oxplow_app::lens_actions::run_lens_action(
            &self.services,
            &root,
            oxplow_app::lens_actions::LensActionCall {
                lens_id: p.id.clone(),
                action_id: p.action,
                params: overrides,
                row,
                input: None,
                on_behalf_of,
                // An agent never confirms.
                confirmed: false,
            },
            &ctx,
        )
        .await
        .map_err(command_error)?;
        json_result(&out)
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
        let p = params.0;
        check_optional_stream("validate_extension", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        let report = oxplow_sdk::check(
            &root,
            &p.name,
            &self.services.extension_catalog,
            Some(&self.services.sql),
            Some(self.services.commands.as_ref()),
            None,
        )
        .await
        .map_err(|e| match e {
            oxplow_sdk::SdkError::NotFound(_) => McpError::invalid_params(
                format!(
                    "no extension `{}` under oxplow/extensions/ in that stream",
                    p.name
                ),
                None,
            ),
            other => internal(other),
        })?;
        json_result(&report)
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
        let p = params.0;
        check_optional_stream("preview_extension", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream.
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self
            .services
            .worktrees
            .resolve(stream.as_deref())
            .await
            .into_local_path();
        // An agent's preview: the extension's programs see no environment
        // (no person's credentials), and model calls answer from the fixtures.
        let report = oxplow_sdk::extension_test::test_extension_in(
            &root,
            &p.name,
            false,
            std::sync::Arc::new(|_: &str| None),
        )
        .await
        .map_err(|e| match e {
            oxplow_sdk::SdkError::NotFound(_) => McpError::invalid_params(
                format!(
                    "no extension `{}` under oxplow/extensions/ in that stream",
                    p.name
                ),
                None,
            ),
            other => internal(other),
        })?;
        json_result(&report)
    }

    #[tool(
        description = "Read what a ref names: its model's row (every column), what sits beside \
                       it (a work item's links and comments, a commit's or an effort's files) \
                       and, for a body no model holds, its content. `file:src/a.rs` reads the \
                       working tree; `file:src/a.rs@git:HEAD#L10-40` a revision's lines \
                       (`@snap:<id>` a local-history snapshot); `event:<id>#output` (or \
                       `#input`, `#prompt`) one of your stream's events' stored bodies; \
                       `lens:<ext>/<slug>` a lens's definition. Any `ref` a model's column \
                       holds reads the same way (`work_item:…`, `commit:…`, `effort:…`, \
                       `thread:…`, `wiki:…`). Content stops at 64 KB, saying so."
    )]
    async fn read(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ReadParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_app::ref_read::{read, ReadError};
        let stream = caller_of(&extensions).stream_id;
        let out = read(&self.services, &params.0.reference, stream)
            .await
            .map_err(|e| match e {
                ReadError::Storage(e) => internal(e),
                other => McpError::invalid_params(other.to_string(), None),
            })?;
        json_result(&out)
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
        params: Parameters<QuerySqlParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let out = self
            .services
            .sql
            .clone()
            .query_sql(
                &p.sql,
                p.params
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                p.limit.map(|l| l as usize),
            )
            .await
            .map_err(|e| match e {
                oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
                other => internal(other),
            })?;
        json_result(&out)
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
        params: Parameters<SiteSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("search", params.0.stream_id.as_deref())?;
        let kinds = params.0.kinds.unwrap_or_default();
        let hits = self
            .services
            .search_store
            .search(
                &params.0.query,
                params.0.stream_id.as_deref(),
                &kinds,
                params.0.limit.unwrap_or(50) as usize,
            )
            .await
            .map_err(internal)?;
        json_result(&hits)
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
        description = "Git working-tree status: per-file change scopes for the \
                          worktree (omit stream_id for the current worktree)."
    )]
    async fn git_status(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<GitStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("git_status", params.0.stream_id.as_deref())?;
        let sid = self
            .stream_or_callers(&caller_of(&extensions), params.0.stream_id)
            .await;
        let ws = self
            .services
            .worktrees
            .resolve(sid.as_deref())
            .await
            .into_local_path();
        let scopes = self
            .services
            .git
            .change_scopes(&ws)
            .await
            .map_err(|e| domain_err(e.into()))?;
        json_result(&scopes)
    }

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
        let p = params.0;
        check_optional_stream("vcs_blame", p.stream_id.as_deref())?;
        let sid = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id)
            .await;
        let revision = parse_revision(p.revision.as_deref().unwrap_or("working"))?;
        let lines =
            oxplow_app::vcs::reads::blame(&self.services, sid.as_deref(), &p.path, &revision)
                .await
                .map_err(domain_err)?;
        json_result(&lines)
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
        self.diff_as(&caller_of(&extensions), params.0).await
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

    #[tool(
        description = "List the project's architectural ZONE table (the `zones:` block in \
            .oxplow/project.yaml) plus what it actually matches: a file count per zone across \
            the worktree, and a sample of paths that fell through to `other`. Zones are how \
            Change-analysis groups churn and flags cross-boundary imports. Oxplow ships NO \
            built-in table — an empty `rules` means this project hasn't declared its zones yet \
            and every file reads as `other`. A large `other` count or an unmatched sample full \
            of real source is the signal the table has gone stale as the repo grew. To change \
            the table, set the `zones` key with the `oxplow.config.set` command (`run_command`)."
    )]
    async fn list_zones(&self) -> Result<CallToolResult, McpError> {
        let report = oxplow_app::zones_service::zone_report(&self.services)
            .await
            .map_err(internal)?;
        json_result(&report)
    }

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
        params: Parameters<ListCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let threads =
            match resolve_comment_scope(params.0.scope.as_deref(), params.0.id.as_deref())? {
                CommentScope::Thread(id) => self
                    .services
                    .comment_store
                    .list_for_thread(&id)
                    .await
                    .map_err(internal)?,
                CommentScope::Stream(id) => self
                    .services
                    .comment_store
                    .list_for_stream(&id)
                    .await
                    .map_err(internal)?,
            };
        let status = params.0.status.as_deref().unwrap_or("all");
        let filtered: Vec<_> = threads
            .into_iter()
            .filter(|t| match status {
                "needs_response" => t.needs_response(),
                "open" => t.comment.status == CommentStatus::Open,
                _ => true,
            })
            .collect();
        // Hydrate the typed context the comment was anchored in so the
        // agent sees *what the highlighted thing is* — the primary
        // target, the nesting of regions it sat inside, and any refs
        // inside the selection — in this one tool call.
        let mut enriched = Vec::with_capacity(filtered.len());
        for t in filtered {
            let primary = ref_resolver::resolve_ref(
                &self.services,
                &t.comment.target_kind,
                &t.comment.target_id,
            )
            .await;
            let context_chain =
                ref_resolver::resolve_refs(&self.services, &t.comment.context_chain).await;
            let referenced =
                ref_resolver::resolve_refs(&self.services, &t.comment.referenced_refs).await;
            enriched.push(EnrichedCommentThread {
                thread: t,
                primary,
                context_chain,
                referenced,
            });
        }
        json_result(&enriched)
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
        params: Parameters<WikiRefDriftParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let drift = oxplow_app::wiki_drift::compute_wiki_ref_drift(
            &self.services.page_ref_store,
            &self.services.snapshot_store,
            &self.services.snapshot_content,
            &self.services.layout.project_dir,
            &p.slug,
            &p.path,
        )
        .await
        .map_err(internal)?;
        json_result(&drift)
    }

    // ---------- task orchestration ----------

    #[tool(
        description = "Unified backlinks: every page (wiki, task, commit, finding, \
                       …) that points AT the given target page. The target is identified \
                       by its ref `kind` (\"file\", \"wiki\", \"work_item\", \"commit\", \
                       \"finding\", \"dir\") and `id` (path / slug / oxplow:tsk… / sha). \
                       Returns one row per inbound edge, including ref_type so the caller \
                       can distinguish e.g. a commit's touched_file edge from a wiki body \
                       mention."
    )]
    async fn list_backlinks(
        &self,
        params: Parameters<PageRefParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .services
            .page_ref_store
            .list_backlinks(&p.kind, &p.id, Some(p.limit as i64))
            .await
            .map_err(internal)?;
        json_result(&edges)
    }

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
        description = "The host services an extension's Starlark script may use, each a scope \
                       it declares in its `needs`: what it gives, the builtins it provides, \
                       whether a person approves it (a model spends their key) and the runs \
                       that may use it (a script that composes runs pure: no model)."
    )]
    async fn list_services(&self) -> Result<CallToolResult, McpError> {
        json_result(&oxplow_domain::service::SERVICES)
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
        description = "Unified outbound: every page the given source page points AT. \
                       Inverse of `list_backlinks` — ask \"what does THIS page reference?\". \
                       Same `kind`/`id` shape as list_backlinks."
    )]
    async fn list_outbound(
        &self,
        params: Parameters<PageRefParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .services
            .page_ref_store
            .list_outbound(&p.kind, &p.id, Some(p.limit as i64))
            .await
            .map_err(internal)?;
        json_result(&edges)
    }

    #[tool(
        description = "Where the symbol at a position is defined — typed locations \
                       (`{ path, range: { start: { line, col }, end } }`, 1-based; paths \
                       workspace-relative, absolute outside it), from the file's language \
                       server. See `lsp_list_servers` for coverage."
    )]
    async fn code_definition(
        &self,
        params: Parameters<CodePositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let at = code_position(
            &params.0.stream_id,
            &params.0.path,
            params.0.line,
            params.0.col,
        )?;
        json_result(
            &self
                .services
                .code_intel
                .definition(&at)
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "Every reference to the symbol at a position (typed locations, \
                       1-based), the declaration included unless `include_declaration` is \
                       false."
    )]
    async fn code_references(
        &self,
        params: Parameters<CodeReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
        json_result(
            &self
                .services
                .code_intel
                .references(&at, p.include_declaration.unwrap_or(true))
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "The hover for a position — the symbol's type/signature and docs, as \
                       markdown (`{ contents, range }`), or null."
    )]
    async fn code_hover(
        &self,
        params: Parameters<CodePositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let at = code_position(
            &params.0.stream_id,
            &params.0.path,
            params.0.line,
            params.0.col,
        )?;
        json_result(
            &self
                .services
                .code_intel
                .hover(&at)
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "The symbols a file declares (functions, classes, methods, modules, …) \
                       via its language server — each `{ name, kind, container, location }`, \
                       nested ones naming their container. Covers any configured LSP \
                       language; `list_code_units` is the offline tree-sitter counterpart."
    )]
    async fn code_symbols(
        &self,
        params: Parameters<CodeFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let stream = code_stream(&params.0.stream_id)?;
        json_result(
            &self
                .services
                .code_intel
                .document_symbols(stream, &params.0.path)
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "Find symbols across the workspace by name (fuzzy, server-defined) from \
                       one language's server — typed symbols with their locations. Use to find \
                       a definition by name without a file/position."
    )]
    async fn code_workspace_symbols(
        &self,
        params: Parameters<CodeWorkspaceSymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let stream = code_stream(&p.stream_id)?;
        json_result(
            &self
                .services
                .code_intel
                .workspace_symbols(stream, &p.language, &p.query)
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "The symbol at a position's callers (`direction: incoming`) or callees \
                       (`outgoing`): each `{ symbol, at }` — the caller/callee and where the \
                       calls are. [] when the position has no call-hierarchy item."
    )]
    async fn code_call_hierarchy(
        &self,
        params: Parameters<CodeCallHierarchyParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let at = code_position(&p.stream_id, &p.path, p.line, p.col)?;
        json_result(
            &self
                .services
                .code_intel
                .call_hierarchy(&at, p.direction)
                .await
                .map_err(code_err)?,
        )
    }

    #[tool(
        description = "List the code units in a file — functions, classes, modules, and (for \
                       languages that have them, e.g. Go/Java) the package — via oxplow's \
                       built-in tree-sitter analysis. Deterministic and offline; works for the \
                       bundled languages (Rust, TS/TSX, JS, Python, Go, Java, C, C++, Clojure, \
                       C#). Complements code_symbols (which covers any LSP language). \
                       Each unit: kind, name, containerPath, startLine, endLine. Empty for \
                       unsupported / unparseable files."
    )]
    async fn list_code_units(
        &self,
        params: Parameters<ListCodeUnitsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let file = self
            .services
            .workspace_files
            .read(Some(p.stream_id.as_str()), p.path.clone())
            .await
            .map_err(|e| internal(e.to_string()))?;
        let units: Vec<serde_json::Value> = oxplow_code_metrics::list_units(&p.path, &file.content)
            .iter()
            .map(|u| {
                serde_json::json!({
                    "kind": u.kind.as_str(),
                    "name": u.name,
                    "containerPath": u.container_path,
                    "startLine": u.start_line,
                    "endLine": u.end_line,
                })
            })
            .collect();
        json_result(&units)
    }

    #[tool(
        description = "List every configured language server (.oxplow/project.yaml + Mason-installed): \
                       languageId, command, source, binary presence, running streams. Use to \
                       check what LSP coverage exists before code_hover/definition/references, \
                       and to verify an `oxplow.lsp.install_server` took effect."
    )]
    async fn lsp_list_servers(&self) -> Result<CallToolResult, McpError> {
        let listings = self.services.lsp_sessions.list_servers().await;
        json_result(&listings)
    }
}

fn code_stream(stream_id: &str) -> Result<StreamId, McpError> {
    expect_id_kind("code", "stream_id", stream_id, ID_STREAM)?;
    StreamId::try_from_str(stream_id)
        .ok_or_else(|| McpError::invalid_params(format!("`{stream_id}` is not a stream id"), None))
}

fn code_position(
    stream_id: &str,
    path: &str,
    line: u32,
    col: u32,
) -> Result<oxplow_domain::code_intel::Position, McpError> {
    if line == 0 || col == 0 {
        return Err(McpError::invalid_params(
            "line and col are 1-based".to_string(),
            None,
        ));
    }
    Ok(oxplow_domain::code_intel::Position {
        stream: code_stream(stream_id)?,
        path: path.to_string(),
        line,
        col,
    })
}

/// A missing or stopped server is the caller's to fix (install,
/// configure or start one); the message says how.
fn code_err(e: oxplow_domain::code_intel::CodeIntelError) -> McpError {
    match e {
        oxplow_domain::code_intel::CodeIntelError::NoProvider(m)
        | oxplow_domain::code_intel::CodeIntelError::NotRunning(m) => {
            McpError::invalid_params(m, None)
        }
        other => internal(other.to_string()),
    }
}

/// Tools that only READ state — annotated `read_only_hint` so a client can
/// auto-approve them instead of prompting (tsk203). Kept as an explicit set
/// (rather than 100 per-`#[tool]` annotations) so the read/write split lives in
/// one reviewable place; `read_write_split_covers_every_tool` fails if a new
/// tool isn't classified here or in [`WRITE_TOOLS`].
const READ_ONLY_TOOLS: &[&str] = &[
    "get_skill",
    "list_collectors",
    "list_ai_roles",
    "get_open_page",
    "list_extensions",
    "list_lenses",
    "run_lens",
    "validate_extension",
    "preview_extension",
    "query_sql",
    "search",
    "git_status",
    "vcs_blame",
    "diff",
    "read",
    "list_zones",
    "list_comments",
    "wiki_ref_drift",
    "list_backlinks",
    "list_outbound",
    "list_commands",
    "list_services",
    "code_definition",
    "code_hover",
    "code_references",
    "code_symbols",
    "code_workspace_symbols",
    "list_code_units",
    "code_call_hierarchy",
    "lsp_list_servers",
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
    "run_collector",
    // A derived cache: the change's analysis, recomputed from the VCS on
    // demand (ipc-and-stores.md, "what stays off the bus").
    "ensure_change",
    // Runs a source's program; stores nothing.
    "preview_collector",
    // Clones into .oxplow/tmp; stores nothing.
    "review_extension",
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

impl OxplowMcp {
    /// The context a lens runs in for an agent: the given stream and
    /// thread (checked), defaulting as [`oxplow_app::extensions::lens_context`] does.
    async fn lens_context(
        &self,
        stream_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<oxplow_app::extensions::LensContext, McpError> {
        let stream = stream_id.map(parse_stream_id).transpose()?;
        let thread = match thread_id {
            Some(t) => {
                expect_id_kind("run_lens", "thread_id", t, ID_THREAD)?;
                Some(parse_thread_id(t)?)
            }
            None => None,
        };
        Ok(oxplow_app::extensions::lens_context(&self.services, stream, thread).await)
    }
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

/// AI failures the agent can act on (no model assigned, bad key, provider
/// error) are invalid-params with the explanation; keychain trouble is internal.
fn ai_error(e: oxplow_app::ai_service::AiServiceError) -> McpError {
    match e {
        oxplow_app::ai_service::AiServiceError::Secret(_) => internal(e),
        _ => McpError::invalid_params(e.to_string(), None),
    }
}

fn internal<E: std::fmt::Display>(e: E) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// Map an extension install/update error to an MCP error.
fn extension_error(e: oxplow_domain::DomainError) -> McpError {
    match e {
        oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
        other => internal(other),
    }
}

/// Map a lens lookup/run error to an MCP error an agent can act on.
fn lens_error(id: &str, e: oxplow_domain::DomainError) -> McpError {
    match e {
        oxplow_domain::DomainError::NotFound => McpError::invalid_params(
            format!("no lens `{id}` (ids are `<extension>/<slug>`; see list_lenses)"),
            None,
        ),
        oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
        other => internal(other),
    }
}

/// Validate an optional `stream_id`: enforce the `s-` prefix when present,
/// and accept `None` (resolves to the current/primary worktree downstream).
/// A `Revision` a tool was given (`working`, `snap:<id>`, `git:<rev>`).
fn parse_revision(raw: &str) -> Result<oxplow_domain::vcs::Revision, McpError> {
    raw.parse()
        .map_err(|e: String| McpError::invalid_params(e, None))
}

/// A domain failure as a tool error: bad input is the caller's.
fn domain_err(e: oxplow_domain::DomainError) -> McpError {
    match e {
        oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
        other => internal(other),
    }
}

fn check_optional_stream(tool: &str, stream_id: Option<&str>) -> Result<(), McpError> {
    match stream_id {
        Some(id) => expect_id_kind(tool, "stream_id", id, ID_STREAM),
        None => Ok(()),
    }
}

/// The resolved target of a `list_comments` call — either a single
/// thread or a whole stream (workspace).
#[derive(Debug, PartialEq, Eq)]
enum CommentScope {
    Thread(ThreadId),
    Stream(StreamId),
}

/// Resolve the `(scope, id)` pair `list_comments` was called with into a
/// concrete [`CommentScope`].
///
/// `scope` is optional: when omitted it's inferred from `id`'s prefix
/// (`thr…` → thread, `str…` → stream). This keeps weaker models from
/// thrashing on a `missing field "scope"` transport error when the id
/// alone already determines the scope. When `scope` *is* given it's
/// honored and the id must match it. Every failure path returns an
/// agent-readable [`McpError`] naming the fix rather than a raw -32602.
fn resolve_comment_scope(scope: Option<&str>, id: Option<&str>) -> Result<CommentScope, McpError> {
    let id = id.map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
        McpError::invalid_params(
            "list_comments: pass `id` — a thread id (`thr…`) or stream id (`str…`)",
            None,
        )
    })?;

    // Normalize an explicit scope up front so an unknown string is caught
    // before we look at the id.
    let scope = scope.map(str::trim).filter(|s| !s.is_empty());
    let want = match scope {
        Some("thread") => Some(ID_THREAD),
        Some("stream") => Some(ID_STREAM),
        Some(other) => {
            return Err(McpError::invalid_params(
                format!(
                    "list_comments: `scope` must be \"thread\" or \"stream\", got `{other}` \
                     (or omit it and I'll infer it from `id`)"
                ),
                None,
            ));
        }
        None => None,
    };

    match want {
        // Explicit scope: validate the id matches, then build it.
        Some(ID_THREAD) => {
            expect_id_kind("list_comments", "id", id, ID_THREAD)?;
            Ok(CommentScope::Thread(parse_thread_id(id)?))
        }
        Some(ID_STREAM) => {
            expect_id_kind("list_comments", "id", id, ID_STREAM)?;
            Ok(CommentScope::Stream(parse_stream_id(id)?))
        }
        Some(_) => unreachable!("want is only ever ID_THREAD/ID_STREAM/None"),
        // No scope: infer it from the id's prefix.
        None => match id.parse::<oxplow_domain::AnyId>() {
            Ok(any) if any.kind.prefix() == ID_THREAD.prefix => {
                Ok(CommentScope::Thread(parse_thread_id(id)?))
            }
            Ok(any) if any.kind.prefix() == ID_STREAM.prefix => {
                Ok(CommentScope::Stream(parse_stream_id(id)?))
            }
            _ => Err(McpError::invalid_params(
                format!(
                    "list_comments: couldn't infer `scope` from id `{id}` — pass scope \
                     \"thread\" (with a `thr…` id) or \"stream\" (with a `str…` id)"
                ),
                None,
            )),
        },
    }
}

/// Validate that a caller-supplied id string carries the expected
/// `<prefix>-…` shape. When the prefix mismatches a known one, return
/// an `invalid_params` error that names the tool/parameter, the value
/// passed, the kind it was inferred to be, and the kind expected. This
/// converts opaque downstream FK-violation errors into actionable
/// guidance at the protocol boundary.
fn parse_stream_id(value: &str) -> Result<StreamId, McpError> {
    StreamId::try_from_str(value)
        .ok_or_else(|| McpError::invalid_params(format!("invalid stream id `{value}`"), None))
}
fn parse_thread_id(value: &str) -> Result<ThreadId, McpError> {
    ThreadId::try_from_str(value)
        .ok_or_else(|| McpError::invalid_params(format!("invalid thread id `{value}`"), None))
}

/// String-id prefix validator. Every external id is now a
/// `<3-letter-prefix><int>` string (e.g. `thr21`); this helper confirms
/// a caller-supplied value parses to the [`EntityKind`] the tool wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdPrefix {
    pub prefix: &'static str,
    pub label: &'static str,
}

pub(crate) const ID_STREAM: IdPrefix = IdPrefix {
    prefix: "str",
    label: "stream id (str…)",
};
pub(crate) const ID_THREAD: IdPrefix = IdPrefix {
    prefix: "thr",
    label: "thread id (thr…)",
};

fn expect_id_kind(
    tool: &str,
    param: &str,
    value: &str,
    expected: IdPrefix,
) -> Result<(), McpError> {
    // Parse against the canonical id grammar. A value that parses to the
    // expected kind passes; anything else gets a corrective message that
    // names what it *looks* like so the caller can fix an "I passed a
    // thread id where a stream id was expected" mix-up in one round-trip.
    match value.parse::<oxplow_domain::AnyId>() {
        Ok(any) if any.kind.prefix() == expected.prefix => Ok(()),
        Ok(any) => Err(McpError::invalid_params(
            format!(
                "{tool}: `{param}` expects a {expected_label}, but got `{value}` which looks like \
                 a {actual_label}",
                expected_label = expected.label,
                actual_label = any.kind.label(),
            ),
            None,
        )),
        Err(_) => Err(McpError::invalid_params(
            format!(
                "{tool}: `{param}` expects a {expected_label}, but got `{value}` which isn't a \
                 valid id",
                expected_label = expected.label,
            ),
            None,
        )),
    }
}

impl OxplowMcp {
    /// The stream a stream-scoped read acts on when the call names none:
    /// the caller's (its header, else its thread's stream); the primary
    /// only for an anonymous caller.
    /// `given`, else the caller's stream ([`Self::caller_stream`]).
    async fn stream_or_callers(&self, caller: &McpCaller, given: Option<String>) -> Option<String> {
        match given {
            Some(s) => Some(s),
            None => self.caller_stream(caller).await,
        }
    }

    async fn caller_stream(&self, caller: &McpCaller) -> Option<String> {
        use oxplow_domain::stores::ThreadStore as _;
        if let Some(s) = caller.stream_id {
            return Some(s.to_string());
        }
        let thread = caller.thread_id?;
        let t = self.services.thread_store.get(&thread).await.ok()??;
        Some(t.stream_id.to_string())
    }

    /// `diff` as `caller`: a stream the call doesn't name is the caller's.
    pub async fn diff_as(
        &self,
        caller: &McpCaller,
        mut p: DiffParams,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("diff", p.stream_id.as_deref())?;
        p.stream_id = self.stream_or_callers(caller, p.stream_id.take()).await;
        let to = parse_revision(&p.to)?;
        let mut from = p.from.as_deref().map(parse_revision).transpose()?;
        let sid = p.stream_id.as_deref();
        if p.since_fork {
            let Some(base) = &from else {
                return Err(McpError::invalid_params("since_fork needs `from`", None));
            };
            let head = oxplow_domain::vcs::Revision::from_rev_slot(Some("git:HEAD"))
                .map_err(|e| McpError::invalid_params(e, None))?;
            let fork_of = if to == oxplow_domain::vcs::Revision::Working {
                &head
            } else {
                &to
            };
            from = oxplow_app::vcs::reads::merge_base(&self.services, sid, base, fork_of)
                .await
                .map_err(domain_err)?;
        }
        let ws = self.services.worktrees.resolve(sid).await.into_local_path();
        let entries = self
            .services
            .trees
            .diff(&ws, from.as_ref(), &to)
            .await
            .map_err(domain_err)?;
        json_result(&entries)
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

/// An extension in `list_extensions`'s summary: its scalar fields and
/// load errors/warnings as they are, every list it contributes counted.
/// The full listing (the bundled extension's ran to 124k characters) is a
/// `name` away.
/// An extension as `list_extensions` summarizes it: its own fields, its
/// errors and warnings, and how many of each kind it contributes, counted
/// from its contributions' descriptors and keyed by the manifest key
/// (`lenses`, `ui.slots`, `event_types`).
fn extension_summary(ext: &oxplow_app::extensions::Extension) -> serde_json::Value {
    let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(ext) else {
        return serde_json::Value::Null;
    };
    let mut out = serde_json::Map::new();
    for (key, value) in fields {
        match value {
            serde_json::Value::Array(items) if key == "errors" || key == "warnings" => {
                out.insert(key, serde_json::Value::Array(items));
            }
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {}
            scalar => {
                out.insert(key, scalar);
            }
        }
    }
    let mut contributes = serde_json::Map::new();
    for d in &ext.contributions {
        let key = d.id.kind.spec().key;
        let n = contributes.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        contributes.insert(key.into(), (n + 1).into());
    }
    out.insert("contributes".into(), serde_json::Value::Object(contributes));
    serde_json::Value::Object(out)
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
        thread: Option<ThreadId>,
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
                .get_open_page(Parameters(ThreadIdParams {
                    thread_id: tid.clone(),
                }))
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
                .get_open_page(Parameters(ThreadIdParams { thread_id: tid }))
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
    async fn source_tools_never_approve_and_schema_lists_extension_entities() {
        use std::os::unix::fs::PermissionsExt;
        let (proj, services, server) = boot().await;
        let ext = proj.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: my-gh\nintent:\n  purpose: test\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        )
        .unwrap();
        let script = ext.join("sync.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"entities\":{\"pr\":[{\"number\":1}]}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        services
            .extension_activation
            .activate(services.clone())
            .await
            .unwrap();

        let list: serde_json::Value =
            serde_json::from_str(&text_payload(server.list_collectors().await.unwrap())).unwrap();
        assert_eq!(list[0]["approved"], false);

        // An agent can't consent on the human's behalf.
        let err = server
            .run_collector(
                as_writer(&services).await,
                Parameters(RunCollectorParams {
                    collector: "collector:my-gh/gh".into(),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("approval"), "{err:?}");
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

        let list = |name: Option<&str>| {
            let server = &server;
            let name = name.map(str::to_string);
            async move {
                serde_json::from_str::<serde_json::Value>(&text_payload(
                    server
                        .list_extensions(
                            rmcp::model::Extensions::new(),
                            Parameters(ListExtensionsParams {
                                stream_id: None,
                                name,
                            }),
                        )
                        .await
                        .unwrap(),
                ))
                .unwrap()
            }
        };
        // The list is a summary: what each extension contributes, counted,
        // and its errors — not every lens's query.
        let all = list(None).await;
        let exts: Vec<_> = all
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["name"] == "demo")
            .collect();
        let exts = serde_json::json!(exts);
        assert_eq!(exts[0]["name"], "demo");
        assert_eq!(exts[0]["contributes"]["lenses"], 3, "{exts}");
        // Every kind counts, the nested ones too (a slot mount under `ui`).
        assert_eq!(exts[0]["contributes"]["ui.slots"], 1, "{exts}");
        assert!(exts[0].get("contributions").is_none(), "{exts}");
        assert!(!exts.to_string().contains("v_stream"), "{exts}");
        // One extension by name: all of it.
        let demo = list(Some("demo")).await;
        assert!(demo.to_string().contains("v_stream"), "{demo}");
        assert_eq!(demo.as_array().map(Vec::len), Some(1), "{demo}");

        let lenses: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .list_lenses(
                    rmcp::model::Extensions::new(),
                    Parameters(StreamScopeParams { stream_id: None }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        let ids: Vec<&str> = lenses
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect();
        // Project lenses, plus the bundled ones.
        assert!(
            ids.contains(&"demo/broken") && ids.contains(&"demo/streams"),
            "{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i.starts_with("oxplow-bundled/")),
            "{ids:?}"
        );

        // A lens's definition is what `read` gives for its ref.
        let read: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .read(
                    rmcp::model::Extensions::new(),
                    Parameters(ReadParams {
                        reference: "lens:demo/streams".into(),
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
                            Parameters(RunLensParams {
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
                Parameters(ReadParams {
                    reference: "lens:demo/nope".into(),
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
                .query_sql(Parameters(QuerySqlParams {
                    sql: "SELECT kind FROM v_stream WHERE kind = ?1".into(),
                    params: Some(vec![serde_json::json!("primary")]),
                    limit: Some(5),
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["columns"], serde_json::json!(["kind"]));
        assert_eq!(out["rows"], serde_json::json!([["primary"]]));

        let err = server
            .query_sql(Parameters(QuerySqlParams {
                sql: "DELETE FROM task".into(),
                params: None,
                limit: None,
            }))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("read-only") || err.message.contains("SELECT"),
            "{err:?}"
        );

        // The catalog is SQL too: every model's columns are documented.
        let cols: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .query_sql(Parameters(QuerySqlParams {
                    sql: "SELECT name FROM v_model_column WHERE view = 'v_work_item'".into(),
                    params: None,
                    limit: None,
                }))
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
            .list_comments(Parameters(ListCommentsParams {
                scope: Some("stream".into()),
                id: Some(stream.to_string()),
                status: None,
            }))
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
    async fn lsp_list_servers_returns_array() {
        let (_proj, _svc, server) = boot().await;
        let r = server.lsp_list_servers().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert!(parsed.is_array());
    }

    #[tokio::test]
    async fn lsp_no_config_error_is_self_describing_for_agents() {
        let (_proj, services, server) = boot().await;
        let stream_id = services.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
        let err = server
            .code_hover(Parameters(CodePositionParams {
                stream_id,
                path: "src/x.rs".into(),
                line: 1,
                col: 1,
            }))
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
            .get_skill(Parameters(GetSkillParams {
                name: "oxplow-extension".into(),
            }))
            .await
            .unwrap();
        assert!(text_payload(r).contains("# Building oxplow lenses"));
        let err = server
            .get_skill(Parameters(GetSkillParams {
                name: "nope".into(),
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("oxplow-extension"), "{err:?}");
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
                        Parameters(ReadParams {
                            reference: format!("work_item:oxplow:{id}"),
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

    /// The services a script may use, as the reference describes them:
    /// each with its builtins, whether a person approves it, and the runs
    /// that may use it.
    #[tokio::test]
    async fn list_services_describes_what_a_script_may_use() {
        let (_proj, _services, server) = boot().await;
        let listed = server.list_services().await.unwrap();
        let text = listed.content[0].as_text().unwrap().text.clone();
        let services: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        let ai = services
            .iter()
            .find(|s| s["scope"] == "ai.compute")
            .unwrap();
        assert_eq!(ai["approval"], true);
        assert_eq!(ai["modes"], serde_json::json!(["external"]));
        assert!(ai["builtins"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("ai_classify")));
        let scopes: Vec<&str> = services
            .iter()
            .map(|s| s["scope"].as_str().unwrap())
            .collect();
        assert_eq!(
            scopes,
            [
                "jobs.read",
                "extension_config.read",
                "state.read",
                "sql.read",
                "tree.read",
                "ai.compute",
                "ai.discover"
            ]
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
        // Fetched on demand, not every turn: ~3K tokens (every full spec was 164 KB).
        assert!(size < 14_000, "the index is {size} bytes");
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
                Parameters(ReadParams {
                    reference: "file:who.txt".into(),
                }),
            )
            .await
            .unwrap();
        let read: serde_json::Value = serde_json::from_str(&text_payload(out)).unwrap();
        assert_eq!(read["content"], "mine");
    }

    /// tsk574: the extension and lens tools without a `stream_id` read
    /// the calling thread's stream — an agent in a worktree sees the
    /// extension it just wrote there, not the primary's.
    #[tokio::test]
    async fn extension_tools_default_to_the_callers_stream() {
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
        let listed = text_payload(
            server
                .list_extensions(
                    caller(),
                    Parameters(ListExtensionsParams {
                        stream_id: None,
                        name: None,
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(listed.contains("\"mine\""), "{listed}");
        // Without a caller, still the primary's.
        let primary_list = text_payload(
            server
                .list_extensions(
                    rmcp::model::Extensions::new(),
                    Parameters(ListExtensionsParams {
                        stream_id: None,
                        name: None,
                    }),
                )
                .await
                .unwrap(),
        );
        assert!(!primary_list.contains("\"mine\""), "{primary_list}");
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
            .wiki_ref_drift(Parameters(WikiRefDriftParams {
                slug: "intro".into(),
                path: "crates/foo.rs".into(),
            }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("\"status\""), "{body}");
        assert!(body.contains("no_pin"), "expected no_pin: {body}");
        // A path the page doesn't reference at all.
        let r2 = server
            .wiki_ref_drift(Parameters(WikiRefDriftParams {
                slug: "intro".into(),
                path: "crates/zzz.rs".into(),
            }))
            .await
            .unwrap();
        assert!(text_payload(r2).contains("not_a_ref"));
    }

    // ---- Pure helpers: parse_status / parse_priority ----

    // ---- expect_id_kind ----

    #[test]
    fn expect_id_kind_accepts_matching_prefix() {
        assert!(expect_id_kind("tool", "thread_id", "thr123", ID_THREAD).is_ok());
    }

    #[test]
    fn expect_id_kind_error_names_tool_param_value_and_kinds() {
        // A stream id passed where a thread id was expected.
        let err = expect_id_kind("read_at", "thread_id", "str123", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("read_at"), "tool name missing: {msg}");
        assert!(msg.contains("thread_id"), "param name missing: {msg}");
        assert!(msg.contains("str123"), "value missing: {msg}");
        assert!(msg.contains("stream id"), "actual label missing: {msg}");
        assert!(msg.contains("thread id"), "expected label missing: {msg}");
    }

    #[test]
    fn expect_id_kind_unrecognised_id_shape_errors() {
        // No `<prefix><int>` shape at all — should still be flagged.
        let err = expect_id_kind("tool", "id", "no-prefix-shape", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("no-prefix-shape"), "value missing: {msg}");
    }

    // ---- resolve_comment_scope ----

    #[test]
    fn resolve_comment_scope_infers_thread_from_id() {
        let scope = resolve_comment_scope(None, Some("thr7")).unwrap();
        assert_eq!(scope, CommentScope::Thread(ThreadId::new(7)));
    }

    #[test]
    fn resolve_comment_scope_infers_stream_from_id() {
        let scope = resolve_comment_scope(None, Some("str3")).unwrap();
        assert_eq!(scope, CommentScope::Stream(StreamId::new(3)));
    }

    #[test]
    fn resolve_comment_scope_honors_explicit_scope() {
        assert_eq!(
            resolve_comment_scope(Some("thread"), Some("thr1")).unwrap(),
            CommentScope::Thread(ThreadId::new(1))
        );
        assert_eq!(
            resolve_comment_scope(Some("stream"), Some("str1")).unwrap(),
            CommentScope::Stream(StreamId::new(1))
        );
    }

    #[test]
    fn resolve_comment_scope_explicit_scope_rejects_mismatched_id() {
        // scope says thread but a stream id was passed → friendly error.
        let err = resolve_comment_scope(Some("thread"), Some("str1")).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("list_comments"), "tool name missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[test]
    fn resolve_comment_scope_missing_id_is_friendly() {
        let err = resolve_comment_scope(Some("thread"), None).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("list_comments"), "tool name missing: {msg}");
        assert!(msg.contains("`id`"), "names the missing param: {msg}");
        // Blank/whitespace id is treated the same as missing.
        assert!(resolve_comment_scope(None, Some("   ")).is_err());
    }

    #[test]
    fn resolve_comment_scope_uninferable_id_is_friendly() {
        // A valid-but-wrong-kind id (task) can't pick a scope.
        let err = resolve_comment_scope(None, Some("tsk9")).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("infer"), "explains it couldn't infer: {msg}");
        assert!(msg.contains("scope"), "names scope as the fix: {msg}");
        // A garbage id is likewise uninferable, not a transport error.
        assert!(resolve_comment_scope(None, Some("nonsense")).is_err());
    }

    #[test]
    fn resolve_comment_scope_unknown_scope_string_is_friendly() {
        let err = resolve_comment_scope(Some("workspace"), Some("str1")).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("workspace"), "echoes the bad value: {msg}");
        assert!(
            msg.contains("\"thread\"") && msg.contains("\"stream\""),
            "lists valid scopes: {msg}"
        );
    }

    /// `list_ai_roles` shows the roles: the built-ins, then extensions'.
    #[tokio::test]
    async fn ai_roles_are_listed() {
        let (_proj, _services, server) = boot().await;
        let roles: serde_json::Value =
            serde_json::from_str(&text_payload(server.list_ai_roles().await.unwrap())).unwrap();
        // The three built-ins, then oxplow-bundled's `decisions` profile.
        assert_eq!(roles["roles"].as_array().unwrap().len(), 4);
        assert_eq!(roles["roles"][3]["role"], "oxplow-bundled/decisions");
        assert_eq!(roles["roles"][0]["binding"], serde_json::Value::Null);
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
