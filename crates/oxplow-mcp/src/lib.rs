//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Tools are thin handlers that
//! delegate into `oxplow-app` services — we never duplicate business
//! logic between the Tauri command surface and the MCP tool surface.
//!
//! Each tool takes a single `Parameters<T>` argument (rmcp
//! convention); request shapes are defined as `serde + JsonSchema`
//! structs alongside the tool methods.

use oxplow_domain::refs::build::work_item_ref;
use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::*;
use rmcp::{tool, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::ref_resolver::{self, RefSummary};
use oxplow_app::Services;
use oxplow_domain::comment::CommentThread;
use oxplow_domain::stores::{CommentStore, TaskNoteStore, TaskStore, ThreadStore};
use oxplow_domain::{CommentStatus, StreamId, Task, TaskId, TaskPriority, TaskStatus, ThreadId};

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

/// Who is calling: the agent thread (and stream) behind an MCP request.
///
/// Every harness carries it on the HTTP request — `X-Oxplow-Thread` /
/// `X-Oxplow-Stream` headers (ACP, opencode, Claude's per-thread MCP
/// config) or `?thread=…&stream=…` on the endpoint URL (Codex, whose
/// config has no per-session headers). rmcp hands the request's
/// `http::request::Parts` to tools through the call's `Extensions`;
/// [`caller_of`] reads them. A transport that carries neither (stdio) is
/// an anonymous agent: it may read, and `run_command` refuses to write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpCaller {
    pub thread_id: Option<oxplow_domain::ThreadId>,
    pub stream_id: Option<oxplow_domain::StreamId>,
}

impl McpCaller {
    pub fn from_parts(parts: &http::request::Parts) -> Self {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let query = |key: &str| {
            parts.uri.query().and_then(|q| {
                q.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    (k == key && !v.is_empty()).then(|| v.to_string())
                })
            })
        };
        let thread = header("x-oxplow-thread").or_else(|| query("thread"));
        let stream = header("x-oxplow-stream").or_else(|| query("stream"));
        Self {
            thread_id: thread.and_then(|t| t.parse().ok()),
            stream_id: stream.and_then(|s| s.parse().ok()),
        }
    }

    pub fn actor(&self) -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
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
const ANONYMOUS_WRITE: &str = "this MCP connection carries no thread identity (no X-Oxplow-Thread \
    header or ?thread= on the endpoint URL), so it may not run commands; oxplow's own harness \
    configs set it — reconnect through one";

// ---------- request shapes ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunCommandParams {
    /// The command's name, as `list_commands` reports it (`config.set`,
    /// `work_item.transition`).
    pub name: String,
    /// The command's input, matching its `input_schema`.
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StreamIdParams {
    pub stream_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ThreadIdParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTasksParams {
    /// Task status to filter by — one of "ready", "in_progress", "blocked",
    /// "done", "canceled", "archived", or "backlog" (thread-detached tasks).
    /// Optional in the wire shape so a missing value yields a readable
    /// error naming the choices rather than a raw transport -32602.
    status: Option<String>,
    /// Required for all status values except "backlog".
    thread_id: Option<String>,
}

/// Slim task row returned by `list_tasks`. Carries only the fields an
/// agent needs to scan and pick work. Description is truncated to 500
/// chars.
#[derive(Debug, Serialize)]
struct TaskListRow {
    id: String,
    parent_id: Option<String>,
    title: String,
    description: String,
    status: TaskStatus,
    priority: TaskPriority,
    sort_index: i64,
}

fn task_list_row(t: Task) -> TaskListRow {
    let raw = t.description.as_str();
    let description = if raw.chars().count() > 500 {
        let truncated: String = raw.chars().take(500).collect();
        format!("{}…", truncated)
    } else {
        raw.to_string()
    };
    TaskListRow {
        id: t.id.to_string(),
        parent_id: t.parent_id.map(|id| id.to_string()),
        title: t.title,
        description,
        status: t.status,
        priority: t.priority,
        sort_index: t.sort_index,
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TaskIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListEffortObservationsParams {
    /// Effort id to read directly. Omit to use the open effort on
    /// `thread_id`.
    pub effort_id: Option<String>,
    pub thread_id: Option<String>,
    /// Optional kind filter: `test-run` | `diff-coverage` | `static-analysis`.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetDashboardParams {
    /// Dashboard id (`dsh<n>`).
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetOpenEffortParams {
    pub thread_id: String,
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
pub struct SearchParams {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SlugParams {
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WikiRefDriftParams {
    /// Wiki slug (without `.md`).
    pub slug: String,
    /// Repo-relative file path referenced by the page.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddFollowupParams {
    pub thread_id: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FollowupIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AwaitUserParams {
    pub thread_id: String,
    pub question: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetThreadContextParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EpicChildSpec {
    pub title: String,
    /// The child task's prose body (required).
    pub description: String,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DispatchTaskParams {
    /// Thread to dispatch from. Required only when `item_id` is
    /// omitted (it's the thread whose first ready item we pick).
    /// When `item_id` is given the thread is inferred from that
    /// task, so this may be left out.
    pub thread_id: Option<String>,
    /// The specific task to dispatch. When omitted, picks the
    /// first ready item on `thread_id` (mirrors main's
    /// dispatch-without-id shortcut for /work-next composition).
    pub item_id: Option<String>,
    /// Optional extra context appended to the brief — usually
    /// orchestrator notes about how this fits into the larger plan.
    pub extra_context: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FindNotesForNoteParams {
    pub slug: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PageRefParams {
    /// Ref kind: `wiki`, `work_item`, `file`, `dir`, `commit`,
    /// `finding`, `task_note` (see .context/refs.md).
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListDeadLettersParams {
    /// Include `retried` and `discarded` letters too (default: `pending` only).
    #[serde(default)]
    pub all: bool,
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

/// A dimension-equality filter for the metric reads.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct McpDimEq {
    pub key: String,
    pub value: String,
}

/// Optional stream selector shared by the stream-scoped VCS read tools.
/// Omit `stream_id` to read the caller's own stream (the primary only for
/// a caller with no thread identity).
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct GitStreamParams {
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitLogParams {
    /// The stream whose history to read; the caller's when absent.
    pub stream_id: Option<String>,
    /// Max commits to return.
    pub limit: Option<u32>,
    /// Include all branches (`--all`) rather than just the current branch.
    #[serde(default)]
    pub all: bool,
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
pub struct SnapshotStreamParams {
    pub stream_id: String,
    /// Max snapshots to return (default 200).
    pub limit: Option<u32>,
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
    /// The action's id, as the lens declares it (see `get_lens` → `actions`).
    pub action: String,
    /// Param overrides by name, as for `run_lens`.
    pub params: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Stream whose worktree to read the lens from; omit for your own.
    pub stream_id: Option<String>,
    /// Your thread, as for `run_lens`.
    pub thread_id: Option<String>,
    /// For a row action (`row: true` in `get_lens` → `actions`): the row
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
pub struct AiQuestionParam {
    /// `noul` (yes/no: returns the probability of yes), `choice` (pick one
    /// of `options`) or `score` (place it on the ordered `levels`).
    #[serde(rename = "type")]
    pub kind: String,
    /// The question, e.g. "Does this change alter public behavior?".
    pub instructions: String,
    /// For `choice`: the options.
    pub options: Option<Vec<String>>,
    /// For `score`: the levels, lowest first.
    pub levels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AiDecideParams {
    /// What the questions are about: a diff, a description, some text.
    pub state: String,
    /// Questions by a short name you choose (e.g. `risky`).
    pub questions: std::collections::BTreeMap<String, AiQuestionParam>,
    /// Role to use; defaults to `decide`.
    pub role: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AiSummarizeParams {
    pub text: String,
    /// What to focus on, e.g. "risks" or "what changed for users".
    pub focus: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunCollectorParams {
    /// The extension that declares it (its folder under `oxplow/extensions/`).
    pub owner: String,
    /// The collector's `id` in its `collectors:`.
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PreviewCollectorParams {
    /// The extension that declares it (its folder under `oxplow/extensions/`).
    pub owner: String,
    /// The collector's `id` in its `collectors:`.
    pub id: String,
    /// Stream whose worktree to run it from; omit for your own.
    pub stream_id: Option<String>,
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
pub struct SnapshotIdParams {
    /// A `snapshot` id — one whole capture (integer, as
    /// `list_snapshots_for_stream` returns).
    pub snapshot_id: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FileSnapshotIdParams {
    /// A `file_snapshot` id — one captured file row (integer, as
    /// `list_files_for_snapshot` returns).
    pub file_snapshot_id: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EventContentParams {
    /// The event's id (`v_event.id`), e.g. an `agent.tool.finished`.
    pub event_id: String,
    /// Which body: `input` or `output`.
    pub body: oxplow_app::event_bodies::EventBodyKey,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FileAtSnapshotParams {
    /// A `snapshot` id — one whole capture.
    pub snapshot_id: i64,
    /// A repo-relative path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeQualityScanIdParams {
    /// A duplication scan id (from `run_duplication_scan_at`).
    pub scan_id: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SelectThreadMcpParams {
    pub stream_id: String,
    /// Thread to select, or omit/null to clear the selection.
    pub thread_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct SwitchStreamParams {
    /// Stream to make current, or omit/null to clear.
    pub stream_id: Option<String>,
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

    // ---------- liveness / version ----------

    #[tool(description = "Liveness check: returns \"pong\".")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text("pong")]))
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
        match oxplow_plugin::skill_body(&name) {
            Some(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
            None => {
                let names: Vec<&str> = oxplow_plugin::skill_index()
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect();
                Err(McpError::invalid_params(
                    format!("no skill `{name}` (skills: {})", names.join(", ")),
                    None,
                ))
            }
        }
    }

    #[tool(description = "Get the running oxplow daemon version.")]
    async fn app_version(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text(env!(
            "CARGO_PKG_VERSION"
        ))]))
    }

    // ---------- streams ----------

    #[tool(description = "List all streams (primary + worktrees) in this project.")]
    async fn list_streams(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .streams
            .list_streams()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(
        description = "List the user's dashboards (id + title). A dashboard is a grid of metric \
                       tiles. Use `get_dashboard` for one dashboard's tiles (tsk138)."
    )]
    async fn list_dashboards(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .dashboard_store
            .list()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(
        description = "Get one dashboard plus its tiles, in display order. `id` is a `dsh<n>` id."
    )]
    async fn get_dashboard(
        &self,
        params: Parameters<GetDashboardParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = oxplow_domain::DashboardId::try_from_str(&params.0.id)
            .ok_or_else(|| McpError::invalid_params("expected a dashboard id (dsh…)", None))?;
        let got = self
            .services
            .dashboard_store
            .get(id)
            .await
            .map_err(internal)?;
        json_result(&got)
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
            if let (Some(lens_id), Some(root)) = (
                page.page_id.strip_prefix("lens:"),
                worktree_for_thread(&self.services, &thread).await,
            ) {
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
                        &root,
                        lens_id,
                        lens_params,
                        &ctx,
                    )
                    .await?;
                    oxplow_app::lens_text::text_run(&self.services, &root, &run, &ctx).await
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
        let root = self.services.worktrees.resolve(None).await;
        let list = oxplow_app::collector_runner::list_collectors(
            &oxplow_app::collector_runner::Collectors::of(&self.services, &root),
        )
        .await
        .map_err(internal)?;
        json_result(&list)
    }

    #[tool(
        description = "Run an approved extension collector now, refreshing its entities \
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
        // The `collector.sync` command, as the caller: it never approves.
        let actor = self.verified_actor(&caller_of(&extensions)).await?;
        let out = self
            .services
            .commands
            .run(
                &actor,
                oxplow_app::collector_runner::SYNC,
                serde_json::json!({ "owner": p.owner, "id": p.id }),
                false,
            )
            .await
            .map_err(command_error)?;
        json_result(&out.result)
    }

    #[tool(
        description = "Dry-run a collector from a stream's worktree and return the rows it would \
                       store (first 50 per entity, coerced to the declared columns), storing \
                       nothing. Use it to check a collector you're writing, especially in a \
                       worktree stream: run_collector always runs the primary's copy, and \
                       collected data is project-wide. An exec collector still needs a person's \
                       approval of that exact version; starlark/jaq collectors run without it."
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
        let preview = oxplow_app::collector_runner::preview_collector(
            &oxplow_app::collector_runner::Collectors::of(&self.services, &root),
            &p.owner,
            &p.id,
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
                       v_oxplow_analytics_change_interest (look-here-first scores, when \
                       oxplow-analytics is on), v_change_function (added/deleted/modified functions, deltas, \
                       churn), v_change_import (cross-zone imports), \
                       v_oxplow_analytics_change_co_change (files whose usual partners are missing, \
                       or that were dormant; oxplow-analytics), v_change_duplicate (copied blocks; \
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
        description = "List oxplow's AI roles (main, fast, summarize, embed, decide, review): \
                       which provider and model each uses, or none, plus the configured \
                       providers and whether each has a key. Keys are never shown. The person \
                       manages these in Settings → AI."
    )]
    async fn list_ai_roles(&self) -> Result<CallToolResult, McpError> {
        json_result(&self.services.ai.settings().map_err(ai_error)?)
    }

    #[tool(
        description = "Ask a model a typed question about some text, cheaply: yes/no (`noul`, \
                       returns the probability of yes), `choice` among options, or `score` on \
                       ordered levels, each with probabilities. Uses the `decide` role (e.g. \
                       TypeSafe Jev) unless you name another. Good for second opinions like \
                       \"is this diff risky?\". Fails if the role has no model; the person \
                       assigns one in Settings → AI."
    )]
    async fn ai_decide(
        &self,
        params: Parameters<AiDecideParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_app::ai_service::{Question, Role};
        let p = params.0;
        let role: Role = match p.role.as_deref() {
            None => Role::Decide,
            Some(r) => serde_json::from_value(serde_json::json!(r)).map_err(|_| {
                McpError::invalid_params(
                    format!("unknown role `{r}` (main, fast, summarize, embed, decide, review)"),
                    None,
                )
            })?,
        };
        let mut questions = std::collections::BTreeMap::new();
        for (name, q) in p.questions {
            let bad = |m: &str| McpError::invalid_params(format!("question `{name}`: {m}"), None);
            let question = match q.kind.as_str() {
                "noul" => Question::Noul {
                    instructions: q.instructions,
                },
                "choice" => Question::Choice {
                    instructions: q.instructions,
                    options: q
                        .options
                        .filter(|o| !o.is_empty())
                        .ok_or_else(|| bad("a choice needs options"))?,
                },
                "score" => Question::Score {
                    instructions: q.instructions,
                    levels: q
                        .levels
                        .filter(|l| !l.is_empty())
                        .ok_or_else(|| bad("a score needs levels"))?,
                },
                other => return Err(bad(&format!("type `{other}` isn't noul, choice or score"))),
            };
            questions.insert(name, question);
        }
        let decision = self
            .services
            .ai
            .decide(role, "mcp:ai_decide", &p.state, &questions)
            .await
            .map_err(ai_error)?;
        json_result(&decision)
    }

    #[tool(
        description = "Summarize text with oxplow's `summarize` role (a model the person \
                       configured in Settings → AI), optionally with a focus. Useful for long \
                       logs or documents you don't need verbatim."
    )]
    async fn ai_summarize(
        &self,
        params: Parameters<AiSummarizeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let summary = self
            .services
            .ai_compute
            .summarize("mcp:ai_summarize", &p.text, p.focus.as_deref())
            .await
            .map_err(compute_error)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            summary.value,
        )]))
    }

    #[tool(
        description = "Before installing (`git_url`) or updating (`name`) an extension: clone it \
                       and report what it would bring in, installing nothing: the extension as it \
                       would load (lenses, sources with the programs they run, the hosts they \
                       reach and the credentials they read, advisories, collectors; `errors` block \
                       the install), the commit `sha`, and `problems` a dry run of its lenses \
                       found. Show the person what it declares and get their go-ahead, then \
                       pass `sha` as `reviewed_sha` to the `extension.install` / `extension.update` commands \
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
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
                       stream's worktree) with their lenses and any load errors. A lens is a \
                       saved query over the semantic layer plus how to show it; the human sees \
                       each one as a page. To build one, write \
                       `oxplow/extensions/<name>/extension.yaml` and `lenses/<slug>.yaml` with \
                       your normal file tools (see the oxplow-extension skill), then call \
                       `validate_extension`."
    )]
    async fn list_extensions(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<StreamScopeParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("list_extensions", params.0.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), params.0.stream_id.clone())
            .await;
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
        let listed = self.services.listed_extensions(&root).await;
        json_result(&listed)
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
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
        let id: i64 = answer
            .strip_prefix("answer:")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| internal("lens.show returned no answer"))?;
        let (run, text) = oxplow_app::commands::lens::text_answer(&self.services, id)
            .await
            .map_err(domain_err)?;
        json_result(&serde_json::json!({ "answer": answer, "title": run.lens.title, "text": text }))
    }

    #[tool(
        description = "Get one lens's full definition: its SQL query, params with defaults, viz \
                       and column/link settings, and the file it lives in."
    )]
    async fn get_lens(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<LensIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        check_optional_stream("get_lens", p.stream_id.as_deref())?;
        // Omitted: the caller's own stream (tsk574).
        let stream = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id.clone())
            .await;
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
        let lens = self
            .services
            .extension_catalog
            .find_lens(&root, &p.id)
            .map_err(|e| lens_error(&p.id, e))?;
        json_result(&lens)
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
        let overrides = p
            .params
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
            .collect();
        let ctx = self
            .lens_context(stream.as_deref(), p.thread_id.as_deref())
            .await?;
        let run = oxplow_app::extensions::run_lens(
            &self.services.sql,
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
        description = "Run one of a lens's actions (see `get_lens` → `actions`): the command it \
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
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
        description = "Check an extension after editing it (the same report as `oxplow plugin \
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
        let root = self.services.worktrees.resolve(stream.as_deref()).await;
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
        description = "Run ONE read-only SQL statement (`SELECT`/`WITH`) over the published \
                       models — the `v_*` views lenses and the UI read (`v_model` lists them; \
                       physical tables are refused). Joins across views are fine. Metrics read \
                       as columns of a grid: `SELECT bucket, zone, MEASURE('<metric key>') FROM \
                       metric_grid('day'|'week'|'month'[, '<dimension>'])` (keys in \
                       v_metric_spec, dimensions in v_dimension). Positional params `?1`, `?2`, … \
                       bind from `params`. Returns `{columns, rows, truncated, reads}`; rows are \
                       positional arrays. Writes, PRAGMA, ATTACH and multiple statements are \
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
    // Mutations (commit/push/merge/…) intentionally stay on the Bash tool.

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
        let ws = self.services.worktrees.resolve(sid.as_deref()).await;
        let scopes = self
            .services
            .git
            .change_scopes(&ws)
            .await
            .map_err(|e| domain_err(e.into()))?;
        json_result(&scopes)
    }

    #[tool(
        description = "The stream's history from its head, newest first (`all` spans every \
                          branch; `limit` caps the count)."
    )]
    async fn vcs_log(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<GitLogParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        check_optional_stream("vcs_log", p.stream_id.as_deref())?;
        let sid = self
            .stream_or_callers(&caller_of(&extensions), p.stream_id)
            .await;
        let log = oxplow_app::vcs::reads::log(&self.services, sid.as_deref(), p.limit, p.all)
            .await
            .map_err(domain_err)?;
        json_result(&log)
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

    #[tool(
        description = "Read a file as it is in one version of the tree: `working` (on disk), \
                          `snap:<id>` (a local-history snapshot) or `git:<rev>` (a sha, branch, \
                          tag or HEAD). Returns null when the path isn't in that version."
    )]
    async fn read_at(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<ReadAtParams>,
    ) -> Result<CallToolResult, McpError> {
        self.read_at_as(&caller_of(&extensions), params.0).await
    }

    #[tool(
        description = "The project's branches — local and remote-tracking — with their \
                          heads; `is_default` marks the default branch."
    )]
    async fn vcs_branches(&self) -> Result<CallToolResult, McpError> {
        let branches = oxplow_app::vcs::reads::branches(&self.services, None)
            .await
            .map_err(domain_err)?;
        json_result(&branches)
    }

    // ---------- snapshots / local history (read + restore) ----------
    //
    // Thin mirrors of the IPC snapshot reads over `services.snapshot_store` /
    // `blobs`, so the agent can inspect and restore its own change history.
    // (Unlike the UI reads, these don't strip `generated` paths — the agent
    // sees the raw capture history.) The composed dashboard DTOs
    // (`list_file_snapshots`) stay IPC-only. Ids are honest: a `snapshot_id`
    // is a whole capture, a `file_snapshot_id` one captured file row
    // (P2.9); reads and restore share `oxplow_app::snapshot_files`.

    #[tool(description = "List snapshot rows for a stream (one per capture batch), newest first.")]
    async fn list_snapshots_for_stream(
        &self,
        params: Parameters<SnapshotStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_snapshots_for_stream",
            "stream_id",
            &params.0.stream_id,
            ID_STREAM,
        )?;
        let stream_id = oxplow_domain::StreamId::try_from_str(&params.0.stream_id)
            .ok_or_else(|| McpError::invalid_params("invalid stream id", None))?;
        let rows = self
            .services
            .snapshot_store
            .list_snapshots_for_stream(stream_id, params.0.limit.unwrap_or(200) as usize)
            .await
            .map_err(internal)?;
        json_result(&rows)
    }

    #[tool(
        description = "The stream's snapshot operation log, newest first: every take (a new \
                       snapshot, or one that found the tree unchanged) with its trigger \
                       (turn_end, quiet, effort_start, …), parent snapshot, turn/effort anchors, \
                       elapsed time and budget (over_budget when it ran over)."
    )]
    async fn list_snapshot_ops(
        &self,
        params: Parameters<SnapshotStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_snapshot_ops",
            "stream_id",
            &params.0.stream_id,
            ID_STREAM,
        )?;
        let stream_id = oxplow_domain::StreamId::try_from_str(&params.0.stream_id)
            .ok_or_else(|| McpError::invalid_params("invalid stream id", None))?;
        let ops = self
            .services
            .snapshot_store
            .list_ops(stream_id, params.0.limit.unwrap_or(200) as usize)
            .await
            .map_err(internal)?;
        json_result(&ops)
    }

    #[tool(description = "List every file_snapshot row captured under one snapshot id.")]
    async fn list_files_for_snapshot(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let rows = self
            .services
            .snapshot_store
            .list_files_for_snapshot(params.0.snapshot_id)
            .await
            .map_err(internal)?;
        json_result(&rows)
    }

    #[tool(description = "Get one captured file row by its file_snapshot id (null if absent).")]
    async fn get_file_snapshot(
        &self,
        params: Parameters<FileSnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let row = self
            .services
            .snapshot_store
            .get(params.0.file_snapshot_id)
            .await
            .map_err(internal)?;
        json_result(&row)
    }

    #[tool(description = "Created/modified/deleted counts for a snapshot.")]
    async fn get_snapshot_stats(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let stats = self
            .services
            .snapshot_store
            .stats_for_snapshot(params.0.snapshot_id)
            .await
            .map_err(internal)?;
        json_result(&stats)
    }

    #[tool(description = "Per-file change entries for one snapshot (git-log-like shape).")]
    async fn list_snapshot_change_entries(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let rows = self
            .services
            .snapshot_store
            .list_changes_for_snapshot(params.0.snapshot_id)
            .await
            .map_err(internal)?;
        json_result(&rows)
    }

    #[tool(
        description = "Read a captured file row (file_snapshot id) as a (UTF-8 lossy) string. \
                          Null when the row is gone or has no content (oversize, a deletion, \
                          or expired from Local History)."
    )]
    async fn read_file_snapshot(
        &self,
        params: Parameters<FileSnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = self
            .services
            .snapshot_files()
            .read_file_snapshot(params.0.file_snapshot_id)
            .await
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        json_result(&content)
    }

    #[tool(
        description = "Read a stored event body — an `agent.tool.*` event's `input` or \
                          `output` — by the event's id (`v_event.id`) in this connection's \
                          stream: `{text, size, truncated}`, the text capped at 64 KiB \
                          (`truncated` when less than the whole). Null when the event has no \
                          such body or retention removed it."
    )]
    async fn read_event_content(
        &self,
        extensions: rmcp::model::Extensions,
        params: Parameters<EventContentParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_app::event_bodies::{read, EventBodyError};
        let actor = self.verified_actor(&caller_of(&extensions)).await?;
        // An agent reads its own stream's bodies only.
        let oxplow_domain::Actor::Agent {
            stream_id: Some(stream),
            ..
        } = actor
        else {
            return Err(internal("a verified caller is an agent with a stream"));
        };
        let p = params.0;
        let body = read(&self.services, &p.event_id, p.body, Some(stream))
            .await
            .map_err(|e| match e {
                EventBodyError::Storage(e) => internal(e),
                other => McpError::invalid_params(other.to_string(), None),
            })?;
        json_result(&body)
    }

    #[tool(
        description = "Read a path as it was at a snapshot (a whole capture, snapshot id) as a \
                          (UTF-8 lossy) string. Null when the path didn't exist then."
    )]
    async fn read_file_at_snapshot(
        &self,
        params: Parameters<FileAtSnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let content = self
            .services
            .snapshot_files()
            .read_file_at_snapshot(p.snapshot_id, &p.path)
            .await
            .map_err(snapshot_file_error)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        json_result(&content)
    }

    // ---------- code quality (duplication) ----------
    //
    // The per-function metrics scan was retired (tsk229) — those signals live in
    // the metric substrate now (`v_metric_spec` via query_sql; `collector.sync`).
    // Duplicate-block detection remains an inherent feature; read its findings
    // here.

    #[tool(description = "List the duplicate-block findings produced by a duplication scan.")]
    async fn list_code_quality_findings(
        &self,
        params: Parameters<CodeQualityScanIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let findings = self
            .services
            .code_quality_store
            .list_findings(params.0.scan_id)
            .await
            .map_err(internal)?;
        json_result(&findings)
    }

    // ---------- UI selection ----------
    //
    // Selection pointers are off the bus (ipc-and-stores.md). Stream-branch
    // checkout stays on Bash (the agent's worktree shell can `git checkout`).

    #[tool(description = "Select (focus) a thread on a stream, or clear the selection.")]
    async fn select_thread(
        &self,
        params: Parameters<SelectThreadMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("select_thread", "stream_id", &params.0.stream_id, ID_STREAM)?;
        if let Some(tid) = &params.0.thread_id {
            expect_id_kind("select_thread", "thread_id", tid, ID_THREAD)?;
        }
        let stream_id = parse_stream_id(&params.0.stream_id)?;
        let thread_id = params
            .0
            .thread_id
            .as_deref()
            .map(parse_thread_id)
            .transpose()?;
        self.services
            .threads
            .select(&stream_id, thread_id.as_ref())
            .await
            .map_err(internal)?;
        json_result(&serde_json::json!({ "ok": true }))
    }

    #[tool(description = "Set the current/active stream (or omit stream_id to clear it).")]
    async fn switch_stream(
        &self,
        params: Parameters<SwitchStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("switch_stream", params.0.stream_id.as_deref())?;
        let id = params
            .0
            .stream_id
            .as_deref()
            .map(parse_stream_id)
            .transpose()?;
        self.services
            .streams
            .set_current(id.as_ref())
            .await
            .map_err(internal)?;
        json_result(&serde_json::json!({ "ok": true }))
    }

    // ---------- threads ----------

    #[tool(description = "List threads attached to the given stream.")]
    async fn list_thread_work(
        &self,
        params: Parameters<StreamIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_thread_work",
            "stream_id",
            &params.0.stream_id,
            ID_STREAM,
        )?;
        let stream_id = parse_stream_id(&params.0.stream_id)?;
        let list = self
            .services
            .thread_store
            .list_for_stream(&stream_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    // ---------- tasks ----------

    #[tool(
        description = "List tasks filtered by status. Pass status = \"backlog\" for thread-detached \
                       backlog items (no thread_id needed). Any other status (\"ready\", \
                       \"in_progress\", \"blocked\", \"done\", \"canceled\", \"archived\") requires \
                       thread_id. Returns a slim representation — description truncated to 500 \
                       chars. Use get_task for the full record."
    )]
    async fn list_tasks(
        &self,
        params: Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let ListTasksParams { status, thread_id } = params.0;
        let status = status.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let status = status.ok_or_else(|| {
            McpError::invalid_params(
                "list_tasks: pass `status` — one of \"ready\", \"in_progress\", \"blocked\", \
                 \"done\", \"canceled\", \"archived\", or \"backlog\"",
                None,
            )
        })?;
        let list = if status == "backlog" {
            self.services
                .task_store
                .list_backlog()
                .await
                .map_err(internal)?
        } else {
            let task_status = str_to_task_status(status)?;
            let tid = thread_id.ok_or_else(|| {
                McpError::invalid_params("thread_id is required for non-backlog status", None)
            })?;
            expect_id_kind("list_tasks", "thread_id", &tid, ID_THREAD)?;
            let tid = parse_thread_id(&tid)?;
            self.services
                .task_store
                .list_by_status_for_thread(&tid, task_status)
                .await
                .map_err(internal)?
        };
        let rows: Vec<TaskListRow> = list.into_iter().map(task_list_row).collect();
        json_result(&rows)
    }

    #[tool(
        description = "Return the next dispatch unit for the orchestrator. If the highest-priority \
                       ready item is an epic, returns the epic and all its ready descendants as one \
                       atomic unit. Otherwise returns all ready non-epic items so you can pick one or \
                       a related cluster to dispatch. Honors `blocks` links — items waiting on a \
                       non-done blocker are skipped. Returns { mode: \"empty\" } when nothing is ready."
    )]
    async fn read_task_options(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "read_task_options",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = parse_thread_id(&params.0.thread_id)?;
        let result = self
            .services
            .tasks
            .read_task_options(&thread_id, &*self.services.task_link_store)
            .await
            .map_err(internal)?;
        json_result(&result)
    }

    #[tool(description = "Get a single task by id.")]
    async fn get_task(&self, params: Parameters<TaskIdParams>) -> Result<CallToolResult, McpError> {
        let id = parse_task_id("get_task", "id", &params.0.id)?;
        let item = self.services.task_store.get(id).await.map_err(internal)?;
        json_result(&item)
    }

    // ---------- thread notes ----------
    //
    // Per-task notes (`add_work_note` / `list_work_notes`) were
    // retired: `effort.summary` already carries "what
    // shipped on this item", so a parallel note table for the same
    // purpose was duplicative. Thread-scoped notes stay — they back
    // the Explore-subagent findings flow.

    #[tool(description = "List thread-scoped notes.")]
    async fn list_thread_notes(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_thread_notes",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = parse_thread_id(&params.0.thread_id)?;
        let notes = self
            .services
            .work_note_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    // ---------- collection (test runs + diff coverage) ----------

    #[tool(
        description = "Discover the thread's currently-open effort. Returns `{ open, effortId, \
            taskId, startedAt, hasStartSnapshot }` — `open:false` (with null ids) when no effort \
            is open. Use this to find the `effortId` for `effort.amend`, to confirm an effort is \
            open before `test.ingest_coverage` / `test.ingest_analysis` / `test.record_run`, and to debug a \
            `no_open_effort` / `no_baseline` outcome (`hasStartSnapshot:false` ⇒ no baseline)."
    )]
    async fn get_open_effort(
        &self,
        params: Parameters<GetOpenEffortParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_db::EffortStore as _;
        expect_id_kind(
            "get_open_effort",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let tid = parse_thread_id(&params.0.thread_id)?;
        let effort = self
            .services
            .effort_store
            .find_open_for_thread(&tid)
            .await
            .map_err(internal)?;
        let payload = match effort {
            Some(e) => serde_json::json!({
                "open": true,
                "effortId": e.id.to_string(),
                "workItem": e.work_item,
                "taskId": e.task_id().map(|t| t.to_string()),
                "startedAt": e.started_at,
                "hasStartSnapshot": e.start_snapshot_id.is_some(),
            }),
            None => serde_json::json!({
                "open": false,
                "effortId": serde_json::Value::Null,
                "taskId": serde_json::Value::Null,
            }),
        };
        json_result(&payload)
    }

    #[tool(
        description = "List collection observations (test-run / diff-coverage / static-analysis) for an effort. \
            Pass `effort_id` to read it directly, or `thread_id` to use that thread's open \
            effort. Optional `kind` filter."
    )]
    async fn list_effort_observations(
        &self,
        params: Parameters<ListEffortObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_db::EffortStore as _;
        let effort_id = match (params.0.effort_id, params.0.thread_id) {
            (Some(e), _) => e,
            (None, Some(t)) => {
                expect_id_kind("list_effort_observations", "thread_id", &t, ID_THREAD)?;
                let tid = parse_thread_id(&t)?;
                match self
                    .services
                    .effort_store
                    .find_open_for_thread(&tid)
                    .await
                    .map_err(internal)?
                {
                    Some(effort) => effort.id.to_string(),
                    None => {
                        return Err(McpError::invalid_params(
                            "no open effort on that thread",
                            None,
                        ))
                    }
                }
            }
            (None, None) => {
                return Err(McpError::invalid_params(
                    "provide effort_id or thread_id",
                    None,
                ))
            }
        };
        let effort = oxplow_domain::EffortId::try_from_str(&effort_id).ok_or_else(|| {
            McpError::invalid_params(format!("`{effort_id}` is not an effort id"), None)
        })?;
        // The stored evidence (the `effort_evidence` asset's rows), not a
        // second computation of it.
        let rows = self
            .services
            .effort_evidence_store
            .list_observations(effort.value(), params.0.kind)
            .await
            .map_err(internal)?;
        json_result(&rows)
    }

    #[tool(
        description = "List the project's architectural ZONE table (the `zones:` block in \
            .oxplow/project.yaml) plus what it actually matches: a file count per zone across \
            the worktree, and a sample of paths that fell through to `other`. Zones are how \
            Change-analysis groups churn and flags cross-boundary imports. Oxplow ships NO \
            built-in table — an empty `rules` means this project hasn't declared its zones yet \
            and every file reads as `other`. A large `other` count or an unmatched sample full \
            of real source is the signal the table has gone stale as the repo grew. To change \
            the table, set the `zones` key with the `config.set` command (`run_command`)."
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
                       `run_command knowledge.reply_comment { comment, body }`; resolve with \
                       `run_command knowledge.update_comment { comment, status: \"resolved\" }`."
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
                       drifted | unchanged | not_a_ref | no_pin | binary.")]
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

    // ---------- followups ----------

    #[tool(description = "Add a followup reminder for a thread.")]
    async fn add_followup(
        &self,
        params: Parameters<AddFollowupParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("add_followup", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let id = parse_thread_id(&params.0.thread_id)?;
        let item = self.services.followups.add(id, params.0.body);
        json_result(&item)
    }

    #[tool(description = "List followups attached to a thread.")]
    async fn list_followups(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_followups",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = parse_thread_id(&params.0.thread_id)?;
        let list = self.services.followups.list_for_thread(&id);
        json_result(&list)
    }

    #[tool(description = "Remove a single followup by id.")]
    async fn remove_followup(
        &self,
        params: Parameters<FollowupIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("remove_followup", "id", &params.0.id, ID_FOLLOWUP)?;
        self.services.followups.remove(&params.0.id);
        Ok(CallToolResult::success(vec![ContentBlock::text("removed")]))
    }

    // ---------- task orchestration ----------

    #[tool(
        description = "Park this thread on the person: logs that the agent is awaiting their answer (the question shows on the rail), so the Stop that follows adds no directive."
    )]
    async fn await_user(
        &self,
        params: Parameters<AwaitUserParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("await_user", "thread_id", &p.thread_id, ID_THREAD)?;
        let tid = parse_thread_id(&p.thread_id)?;
        let question = p.question.trim().to_string();
        // Detail carries the question text (not a bare marker) so the
        // rail agent-status dot can show it in a tooltip. Empty questions
        // fall back to None — the dot still flips to "awaiting you",
        // just without tooltip text.
        let detail = (!question.is_empty()).then(|| question.clone());
        // Park the thread on the person: the status is logged as
        // `agent.status.changed{awaiting_user}` (what the derived status and
        // the Stop pipeline read — P3.9), stored, and announced, so the rail
        // dot turns "awaiting you" (with the question tooltip) immediately.
        self.services
            .hook_ingest
            .set_status(&tid, oxplow_domain::AgentStatusState::AwaitingUser, detail)
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            "awaiting",
        )]))
    }

    #[tool(description = "Bundle of thread state, tasks, and recent activity.")]
    async fn get_thread_context(
        &self,
        params: Parameters<GetThreadContextParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "get_thread_context",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = parse_thread_id(&params.0.thread_id)?;
        let thread = self
            .services
            .thread_store
            .get(&id)
            .await
            .map_err(internal)?;
        let items = self
            .services
            .task_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        let bundle = serde_json::json!({
            "thread": thread,
            "items": items,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            bundle.to_string(),
        )]))
    }

    #[tool(
        description = "Compose a ready-to-paste dispatch brief for a task. With `item_id`, \
                       that item; otherwise the first ready non-epic item on the thread. Returns \
                       `{ ok, prompt, itemId }` — pass `prompt` to the general-purpose Agent tool. \
                       The brief carries the item fields, AC, recent notes, and the subagent \
                       protocol preamble; the sub-agent moves the item to in_progress on entry \
                       (`work_item.transition`)."
    )]
    async fn dispatch_task(
        &self,
        params: Parameters<DispatchTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let parsed_item_id = match params.0.item_id.as_deref() {
            Some(raw) => Some(parse_task_id("dispatch_task", "item_id", raw)?),
            None => None,
        };
        let target = match parsed_item_id {
            // An explicit item id fully determines the work, so the
            // thread is inferred from the task itself — `thread_id` is
            // not needed in this path.
            Some(id) => self
                .services
                .task_store
                .get(id)
                .await
                .map_err(internal)?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("dispatch_task: item not found: {}", id.value()),
                        None,
                    )
                })?,
            // Without an item id we need a thread to pick the first
            // ready item from. Require it here with a corrective error
            // rather than up front, so the item-id-only call works.
            None => {
                let Some(raw_thread) = params.0.thread_id.as_deref() else {
                    return Err(McpError::invalid_params(
                        "dispatch_task: provide either `item_id` (the specific task to \
                         dispatch) or `thread_id` (to dispatch the first ready item on that \
                         thread)"
                            .to_string(),
                        None,
                    ));
                };
                expect_id_kind("dispatch_task", "thread_id", raw_thread, ID_THREAD)?;
                let thread_id = parse_thread_id(raw_thread)?;
                let items = self
                    .services
                    .task_store
                    .list_for_thread(&thread_id)
                    .await
                    .map_err(internal)?;
                // Build a set of task ids that have children → epics.
                let epic_ids: std::collections::HashSet<TaskId> =
                    items.iter().filter_map(|i| i.parent_id).collect();
                let mut ready_first: Vec<_> = items
                    .into_iter()
                    .filter(|i| {
                        matches!(i.status, oxplow_domain::TaskStatus::Ready)
                            && !epic_ids.contains(&i.id)
                    })
                    .collect();
                ready_first.sort_by_key(|i| (i.sort_index, i.created_at));
                let Some(it) = ready_first.into_iter().next() else {
                    return json_result(&serde_json::json!({
                        "ok": false,
                        "reason": "no ready non-epic item on thread",
                    }));
                };
                it
            }
        };

        let prompt =
            compose_dispatch_brief(&target, params.0.extra_context.as_deref().unwrap_or(""));
        json_result(&serde_json::json!({
            "ok": true,
            "prompt": prompt,
            "itemId": target.id,
        }))
    }

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
        description = "The commands this agent may run through `run_command`, each with its \
                       input schema, one-line summary, whether it needs a person's confirmation \
                       and whether it is undoable. Commands are the one write path: every run \
                       is validated, policy-checked, audited and logged as `command.executed`."
    )]
    async fn list_commands(
        &self,
        extensions: rmcp::model::Extensions,
    ) -> Result<CallToolResult, McpError> {
        let actor = caller_of(&extensions).actor();
        json_result(&self.services.commands.list(&actor))
    }

    #[tool(
        description = "Run a command by name with its input (see `list_commands`). Returns \
                       `{ result, audit_id, event_id, inverse? }`. Invalid input names the \
                       failing field; a denied command says why. A command that needs a \
                       person's confirmation is not run: it is recorded as a proposal and \
                       this returns `{ proposal, message }` — tell the person it waits for \
                       them in this thread and in Approvals, and don't run it again; its \
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
        description = "The event log's dead-letter queue: events a consumer failed on, \
                       parked with the error (`pending` by default; `all` includes retried \
                       and discarded). Each row names the consumer, the event and the \
                       failure; the same data is `v_event_dead_letter` in query_sql."
    )]
    async fn list_dead_letters(
        &self,
        params: Parameters<ListDeadLettersParams>,
    ) -> Result<CallToolResult, McpError> {
        let letters = self
            .services
            .event_pump
            .list_dead_letters(params.0.all)
            .await
            .map_err(internal)?;
        json_result(&letters)
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
        description = "What the language servers last reported for a file — typed \
                       diagnostics (`severity`, `message`, `source`, `code`, `range`, \
                       1-based). Every file's are `v_diagnostic` in query_sql."
    )]
    async fn code_diagnostics(
        &self,
        params: Parameters<CodeFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let stream = code_stream(&params.0.stream_id)?;
        json_result(
            &self
                .services
                .code_intel
                .diagnostics(stream, &params.0.path)
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
                       and to verify an `lsp.install_server` took effect."
    )]
    async fn lsp_list_servers(&self) -> Result<CallToolResult, McpError> {
        let listings = self.services.lsp_sessions.list_servers().await;
        json_result(&listings)
    }
}

/// Resolve the per-(stream, language) LspProxy. Helper sitting
/// outside the `#[tool_router]` impl so the macro doesn't try to
/// route it as a tool.
/// Look up the worktree path for a thread by walking
/// thread → stream. Returns `None` when either lookup fails so
/// `record_effort` falls back to the safe default (every touched
/// file → `Updated`). Used to plumb the worktree into
/// `record_effort` so it can stat each touched file and detect
/// deletions.
async fn worktree_for_thread(
    services: &Services,
    thread_id: &oxplow_domain::ThreadId,
) -> Option<std::path::PathBuf> {
    let thread = services.thread_store.get(thread_id).await.ok().flatten()?;
    let streams = services.streams.list_streams().await.ok()?;
    streams
        .into_iter()
        .find(|s| s.id == thread.stream_id)
        .map(|s| std::path::PathBuf::from(s.worktree_path))
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
    "dispatch_task",
    "ping",
    "get_skill",
    "list_collectors",
    "list_ai_roles",
    "get_open_page",
    "list_extensions",
    "list_lenses",
    "get_lens",
    "run_lens",
    "validate_extension",
    "query_sql",
    "app_version",
    "list_streams",
    "list_dashboards",
    "get_dashboard",
    "search",
    "git_status",
    "vcs_log",
    "vcs_blame",
    "diff",
    "read_at",
    "vcs_branches",
    "list_snapshots_for_stream",
    "list_snapshot_ops",
    "list_files_for_snapshot",
    "get_file_snapshot",
    "get_snapshot_stats",
    "list_snapshot_change_entries",
    "read_file_snapshot",
    "read_file_at_snapshot",
    "read_event_content",
    "list_code_quality_findings",
    "list_thread_work",
    "list_tasks",
    "read_task_options",
    "get_task",
    "list_thread_notes",
    "get_open_effort",
    "list_effort_observations",
    "list_zones",
    "list_comments",
    "wiki_ref_drift",
    "list_followups",
    "get_thread_context",
    "list_backlinks",
    "list_outbound",
    "list_dead_letters",
    "list_commands",
    "code_definition",
    "code_hover",
    "code_references",
    "code_symbols",
    "code_workspace_symbols",
    "list_code_units",
    "code_call_hierarchy",
    "code_diagnostics",
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
    // The `lens.show` command (records the answer in the caller's thread).
    "show_lens",
    // The `collector.sync` command.
    "run_collector",
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
    // UI selection pointers and the in-memory follow-ups — off the bus.
    "select_thread",
    "switch_stream",
    "add_followup",
    "remove_followup",
    // Agent-session activity, born as an `agent.*` event.
    "await_user",
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
            "Oxplow MCP server. Exposes task, note, wiki, and stream surfaces \
             for managing oxplow work items and project knowledge."
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

/// A recorded computation's failure: the model's, as [`ai_error`]; an
/// unusable answer the agent can retry; storing it is internal.
fn compute_error(e: oxplow_app::ai_compute::AiComputeError) -> McpError {
    use oxplow_app::ai_compute::AiComputeError;
    match e {
        AiComputeError::Ai(e) => ai_error(e),
        e @ AiComputeError::BadOutput(_) => McpError::invalid_params(e.to_string(), None),
        e @ AiComputeError::Storage(_) => internal(e),
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
/// Parse a task id from its string form. Returns an error suitable for
/// returning straight from a tool handler when the input is not a
/// non-negative integer.
fn parse_task_id(tool: &str, param: &str, value: &str) -> Result<oxplow_domain::TaskId, McpError> {
    let v = value.trim();
    // Accept the prefixed form (`tsk42`) or a bare positive integer (`42`).
    if let Some(id) = oxplow_domain::TaskId::try_from_str(v) {
        return Ok(id);
    }
    if let Ok(n) = v.parse::<i64>() {
        if n > 0 {
            return Ok(oxplow_domain::TaskId::new(n));
        }
    }
    Err(McpError::invalid_params(
        format!("{tool}: `{param}` expects a task id (e.g. `tsk42` or `42`), got `{value}`"),
        None,
    ))
}

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
/// Task ids additionally accept the bare-integer form via
/// [`parse_task_id`].
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
pub(crate) const ID_FOLLOWUP: IdPrefix = IdPrefix {
    prefix: "fup",
    label: "follow-up id (fup…)",
};

fn str_to_task_status(s: &str) -> Result<TaskStatus, McpError> {
    match s {
        "ready" => Ok(TaskStatus::Ready),
        "in_progress" => Ok(TaskStatus::InProgress),
        "blocked" => Ok(TaskStatus::Blocked),
        "done" => Ok(TaskStatus::Done),
        "canceled" => Ok(TaskStatus::Canceled),
        "archived" => Ok(TaskStatus::Archived),
        other => Err(McpError::invalid_params(
            format!(
                "unknown status \"{other}\"; valid values: ready, in_progress, blocked, done, \
                 canceled, archived, backlog"
            ),
            None,
        )),
    }
}

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

/// Compose the brief the orchestrator passes to the general-purpose
/// Agent tool to dispatch a task to a subagent. Pure so it's
/// testable.
///
/// Sections: identity, description, AC, optional extra context, and
/// the closing reminder pointing at the subagent-protocol skill.
/// Per-item notes used to render here too but were retired —
/// effort.summary already records what shipped on prior
/// attempts; reviewers see it from the task activity timeline.
fn compose_dispatch_brief(item: &oxplow_domain::Task, extra_context: &str) -> String {
    let mut out: Vec<String> = vec![
        format!("Task: {}", item.title),
        format!("itemId: {}", item.id.value()),
        format!("priority: {:?}", item.priority),
        String::new(),
    ];
    if !item.description.is_empty() {
        out.push("## Description".into());
        out.push(item.description.clone());
        out.push(String::new());
    }
    if !extra_context.is_empty() {
        out.push("## Extra context".into());
        out.push(extra_context.to_string());
        out.push(String::new());
    }
    out.push("## Protocol".into());
    out.push(format!(
        "Follow the `oxplow-subagent-work-protocol` skill: `work_item.transition` it to \
         in_progress on entry; on exit run the `command.sequence` of `work_item.transition` \
         (done) and `effort.report` with your `touched_files` so Local History attributes the \
         writes. Return ONE line: `oxplow-result: {{\"ok\":true,\"itemId\":\"<id>\",…}}`. \
         If you run tests, run `test.record_run` with `work_item: \"{}\"` — your runs are \
         invisible to oxplow's passive Bash-hook collection, and naming your item attributes \
         them exactly even while sibling efforts are open.",
        work_item_ref(item.id)
    ));
    out.join("\n")
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
        let ws = self.services.worktrees.resolve(sid).await;
        let entries = self
            .services
            .trees
            .diff(&ws, from.as_ref(), &to)
            .await
            .map_err(domain_err)?;
        json_result(&entries)
    }

    /// `read_at` as `caller`: a stream the call doesn't name is the
    /// caller's.
    pub async fn read_at_as(
        &self,
        caller: &McpCaller,
        mut p: ReadAtParams,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("read_at", p.stream_id.as_deref())?;
        p.stream_id = self.stream_or_callers(caller, p.stream_id.take()).await;
        let revision: oxplow_domain::vcs::Revision = p
            .revision
            .parse()
            .map_err(|e: String| McpError::invalid_params(e, None))?;
        let ws = self
            .services
            .worktrees
            .resolve(p.stream_id.as_deref())
            .await;
        let bytes = self
            .services
            .trees
            .read_at(&ws, &revision, &p.path)
            .await
            .map_err(|e| match e {
                oxplow_domain::DomainError::Invalid(m) => McpError::invalid_params(m, None),
                other => internal(other),
            })?;
        json_result(&bytes.map(|b| String::from_utf8_lossy(&b).into_owned()))
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
            .run(&actor, &params.name, params.input, false)
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
        // A write may have opened or closed an effort (`effort.open`,
        // `work_item.transition`): let its snapshot pin land before the
        // agent's next step.
        if outcome.audit_id.is_some() {
            self.services.tasks.settle_lifecycle().await;
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
            thread_id: Some(thread_id),
            stream_id: Some(thread.stream_id),
        })
    }
}

/// A snapshot read/restore failure as the MCP error the agent can act on.
fn snapshot_file_error(err: oxplow_app::snapshot_files::SnapshotFileError) -> McpError {
    use oxplow_app::snapshot_files::SnapshotFileError as E;
    match err {
        E::Other(m) => internal(m),
        other => McpError::invalid_params(other.to_string(), None),
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
        E::Failed { .. } | E::Busy { .. } => internal(err.to_string()),
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
         them in this thread and in Approvals (and on the setting's row in Settings).{replaces} \
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
    use super::*;
    use oxplow_domain::stores::TaskStore;
    use oxplow_domain::task::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};
    use oxplow_domain::time::Timestamp;

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

    fn boot() -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        boot_with_config("")
    }

    /// `boot()` with `project_yaml` written to `.oxplow/project.yaml`
    /// first, so the booted `Services` picks it up (e.g. a
    /// `generated:` block that shapes the workspace filter).
    fn boot_with_config(project_yaml: &str) -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
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
        let services = Arc::new(Services::in_memory(project.path()).unwrap());
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

    fn make_task(thread_id: Option<ThreadId>, title: &str) -> Task {
        let now = Timestamp::now();
        Task {
            id: TaskId::placeholder(),
            thread_id,
            parent_id: None,
            title: title.into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        }
    }

    #[tokio::test]
    async fn server_constructs() {
        let (_proj, _svc, _server) = boot();
    }

    #[tokio::test]
    async fn get_open_page_reports_what_the_human_sees() {
        use oxplow_domain::stores::ThreadStore;
        let (proj, services, server) = boot();
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
            "name: demo\n",
        )
        .unwrap();
        std::fs::write(
            p.join("kinds.yaml"),
            "title: Kinds\nparams:\n  - { name: kind, default: worktree }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        )
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
        let (proj, services, server) = boot();
        let ext = proj.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        )
        .unwrap();
        let script = ext.join("sync.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"entities\":{\"pr\":[{\"number\":1}]}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let list: serde_json::Value =
            serde_json::from_str(&text_payload(server.list_collectors().await.unwrap())).unwrap();
        assert_eq!(list[0]["approved"], false);

        // An agent can't consent on the human's behalf.
        let err = server
            .run_collector(
                as_writer(&services).await,
                Parameters(RunCollectorParams {
                    owner: "my-gh".into(),
                    id: "gh".into(),
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("approval"), "{err:?}");
    }

    #[tokio::test]
    async fn extension_and_lens_tools() {
        let (proj, _services, server) = boot();
        let root = proj.path();
        let w = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w("oxplow/extensions/demo/extension.yaml", "name: demo\n");
        w(
            "oxplow/extensions/demo/lenses/streams.yaml",
            "title: Streams\nparams:\n  - { name: kind, default: primary }\nquery: SELECT kind FROM v_stream WHERE kind = :kind\n",
        );
        w(
            "oxplow/extensions/demo/lenses/broken.yaml",
            "title: Broken\nquery: SELECT x FROM v_nope\n",
        );

        let exts: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .list_extensions(
                    rmcp::model::Extensions::new(),
                    Parameters(StreamScopeParams { stream_id: None }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(exts[0]["name"], "demo");

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
        // Project lenses, plus the bundled oxplow-review ones.
        assert!(
            ids.contains(&"demo/broken") && ids.contains(&"demo/streams"),
            "{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i.starts_with("oxplow-review/")),
            "{ids:?}"
        );

        let lens: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .get_lens(
                    rmcp::model::Extensions::new(),
                    Parameters(LensIdParams {
                        id: "demo/streams".into(),
                        stream_id: None,
                    }),
                )
                .await
                .unwrap(),
        ))
        .unwrap();
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
            .get_lens(
                rmcp::model::Extensions::new(),
                Parameters(LensIdParams {
                    id: "demo/nope".into(),
                    stream_id: None,
                }),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("demo/nope"), "{err:?}");

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
        let (_proj, _services, server) = boot();
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
                    sql: "SELECT name FROM v_model_column WHERE view = 'v_task'".into(),
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
            .any(|r| r[0] == "status"));
    }

    #[tokio::test]
    async fn list_comments_enriches_primary_and_context() {
        let (_proj, services, server) = boot();
        let stream = services.streams.list_streams().await.unwrap()[0].id;

        // A task to use as the primary target, and another as a context
        // ancestor (e.g. an epic the row sat under).
        let primary_task = services
            .task_store
            .insert(&make_task(None, "Primary item"))
            .await
            .unwrap();
        let parent_task = services
            .task_store
            .insert(&make_task(None, "Parent epic"))
            .await
            .unwrap();

        services
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "knowledge.add_comment",
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
        let (_proj, _svc, server) = boot();
        let r = server.lsp_list_servers().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert!(parsed.is_array());
    }

    #[tokio::test]
    async fn lsp_no_config_error_is_self_describing_for_agents() {
        let (_proj, services, server) = boot();
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
        assert!(msg.contains("lsp.install_server"), "got: {msg}");
        assert!(msg.contains("project.yaml"), "got: {msg}");
    }

    #[tokio::test]
    async fn get_info_advertises_tool_capability() {
        let (_proj, _svc, server) = boot();
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }

    #[tokio::test]
    async fn get_info_instructions_have_no_repo_specific_context_path() {
        // The instructions ship to every downstream agent — they must not
        // point at this repo's own `.context/` docs, which don't exist
        // in a user's project.
        let (_proj, _svc, server) = boot();
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
        let (_proj, _svc, server) = boot();
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
        let (proj, svc, server) = boot();
        let ext = proj.path().join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: work\nsources:\n  - id: one\n    runtime: jaq\n    entry: one.jq\n    input: \"SELECT 1 AS n\"\n    entities:\n      - { name: nums, key: n, columns: { n: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("one.jq"), "{entities: {nums: .rows}}").unwrap();
        let r = server
            .preview_collector(
                rmcp::model::Extensions::new(),
                Parameters(PreviewCollectorParams {
                    owner: "work".into(),
                    id: "one".into(),
                    stream_id: None,
                }),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(v["entities"][0]["rows"], serde_json::json!([[1]]));
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ping_returns_pong() {
        let (_proj, _svc, server) = boot();
        let r = server.ping().await.unwrap();
        assert_eq!(text_payload(r), "pong");
    }

    #[tokio::test]
    async fn app_version_returns_cargo_version() {
        let (_proj, _svc, server) = boot();
        let r = server.app_version().await.unwrap();
        assert_eq!(text_payload(r), env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn list_streams_returns_primary_for_fresh_project() {
        // Boot ensures the primary stream exists (snapshot capture is
        // stream-scoped and must always have one), so a freshly booted
        // services has exactly one primary stream.
        let (_proj, _services, server) = boot();
        let r = server.list_streams().await.unwrap();
        let body = text_payload(r);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], "primary");
    }

    #[tokio::test]
    async fn list_backlog_includes_unassigned_items() {
        let (_proj, services, server) = boot();
        let backlog_item = make_task(None, "do the thing");
        let id = services.task_store.insert(&backlog_item).await.unwrap();

        let r = server
            .list_tasks(Parameters(ListTasksParams {
                status: Some("backlog".to_string()),
                thread_id: None,
            }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(
            body.contains(&id.to_string()),
            "backlog item missing from result: {body}",
        );
        assert!(body.contains("do the thing"), "title missing: {body}");
    }

    #[tokio::test]
    async fn get_task_round_trips() {
        let (_proj, services, server) = boot();
        let item = make_task(None, "round trip");
        let id = services.task_store.insert(&item).await.unwrap();

        let r = server
            .get_task(Parameters(TaskIdParams { id: id.to_string() }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("round trip"), "unexpected body: {body}");
    }

    #[tokio::test]
    async fn dispatch_task_infers_thread_from_item_id() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let id = services
            .task_store
            .insert(&make_task(Some(thread.id), "dispatch me"))
            .await
            .unwrap();

        // Only item_id — thread_id is inferred from the task, so a
        // weak model that omits it still succeeds (no -32602).
        let r = server
            .dispatch_task(Parameters(DispatchTaskParams {
                thread_id: None,
                item_id: Some(id.to_string()),
                extra_context: None,
            }))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(parsed["ok"], true);
        assert!(
            parsed["prompt"].as_str().unwrap().contains("dispatch me"),
            "brief should target the item: {parsed}",
        );
    }

    #[tokio::test]
    async fn dispatch_task_requires_thread_or_item() {
        let (_proj, _services, server) = boot();
        let err = server
            .dispatch_task(Parameters(DispatchTaskParams {
                thread_id: None,
                item_id: None,
                extra_context: None,
            }))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("item_id") && msg.contains("thread_id"),
            "error should name both ways to dispatch: {msg}",
        );
    }

    fn parts_with(headers: &[(&str, &str)], uri: &str) -> http::request::Parts {
        let mut b = http::Request::builder().uri(uri);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(()).unwrap().into_parts().0
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
        extensions_for(parts_with(
            &[("x-oxplow-thread", &thread.id.to_string())],
            "http://h/mcp",
        ))
    }

    fn extensions_for(parts: http::request::Parts) -> rmcp::model::Extensions {
        let mut ext = rmcp::model::Extensions::new();
        ext.insert(parts);
        ext
    }

    #[test]
    fn mcp_caller_reads_the_identity_headers_then_the_url_query() {
        let c = McpCaller::from_parts(&parts_with(
            &[("x-oxplow-thread", "thr3"), ("x-oxplow-stream", "str2")],
            "http://h/mcp",
        ));
        assert_eq!(c.thread_id, Some(oxplow_domain::ThreadId::new(3)));
        assert_eq!(c.stream_id, Some(oxplow_domain::StreamId::new(2)));
        assert_eq!(c.actor().source(), "agent:thr3");
        // Codex has no per-session headers: the identity rides the URL.
        let c = McpCaller::from_parts(&parts_with(&[], "http://h/mcp?thread=thr5&stream=str1"));
        assert_eq!(c.thread_id, Some(oxplow_domain::ThreadId::new(5)));
        assert_eq!(c.stream_id, Some(oxplow_domain::StreamId::new(1)));
        // Headers win over the query; garbage is anonymous.
        let c = McpCaller::from_parts(&parts_with(
            &[("x-oxplow-thread", "thr9")],
            "http://h/mcp?thread=thr5",
        ));
        assert_eq!(c.thread_id, Some(oxplow_domain::ThreadId::new(9)));
        let c = McpCaller::from_parts(&parts_with(&[("x-oxplow-thread", "nope")], "http://h/mcp"));
        assert_eq!(c, McpCaller::default());
        assert_eq!(
            caller_of(&rmcp::model::Extensions::new()),
            McpCaller::default()
        );
    }

    #[tokio::test]
    async fn run_command_refuses_an_anonymous_connection_and_lists_for_agents() {
        let (_proj, _services, server) = boot();
        let err = server
            .run_command_as(
                &McpCaller::default(),
                RunCommandParams {
                    name: "config.set".into(),
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
                    name: "metric.record".into(),
                    input: serde_json::json!({"key": "oxplow.rust.unsafe_blocks", "value": 1.0}),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thread identity"), "{err}");
        // Listing needs no identity and shows agent-invocable commands.
        let listed = server
            .list_commands(rmcp::model::Extensions::new())
            .await
            .unwrap();
        let text = listed.content[0].as_text().unwrap().text.clone();
        let specs: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        let names: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        for expected in [
            "config.set",
            "config.list_keys",
            "work_item.transition",
            "metric.record",
            "collector.sync",
            "metric.rebuild",
            "metric.scaffold",
        ] {
            assert!(names.contains(&expected), "{names:?}");
        }
        assert!(specs
            .iter()
            .all(|s| s["input_schema"].is_object() && s["summary"].is_string()));
    }

    /// The thread header is a claim, not a proof: an unknown thread, or a
    /// stream header that isn't the thread's stream, is refused, and
    /// `transition_tasks` needs an identity like `run_command` does.
    #[tokio::test]
    async fn commands_refuse_unknown_or_mismatched_callers() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
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
            name: "config.list_keys".into(),
            input: serde_json::json!({}),
        };
        let err = server
            .run_command_as(
                &McpCaller {
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
    /// `config.set` through `run_command`, and the run is audited to the
    /// calling thread.
    #[tokio::test]
    async fn run_command_sets_zones_as_the_calling_thread() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (proj, services, server) = boot();
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let parts = parts_with(
            &[
                ("x-oxplow-thread", &thread.id.to_string()),
                ("x-oxplow-stream", &stream.id.to_string()),
            ],
            "http://h/mcp",
        );
        let out = server
            .run_command(
                extensions_for(parts),
                Parameters(RunCommandParams {
                    name: "config.set".into(),
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
        assert_eq!(executed.envelope.payload["command"], "config.set");
        // A human-only key is not the agent's to set: the run is kept as a
        // proposal for a person, and the agent is told so — a success,
        // since the request is recorded and needs nothing more from it.
        let out = server
            .run_command_as(
                &McpCaller {
                    thread_id: Some(thread.id),
                    stream_id: Some(stream.id),
                },
                RunCommandParams {
                    name: "config.set".into(),
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
        assert!(message.contains("Approvals"), "{message}");
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

    /// A thread on `stream`, made as a person through `thread.create`.
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

    /// tsk555: `read_at` / `diff` without a `stream_id` read the calling
    /// thread's stream, not the primary's.
    #[tokio::test]
    async fn read_at_and_diff_default_to_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (proj, services, server) = boot();
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
        let caller = McpCaller {
            thread_id: Some(thread.id),
            stream_id: None,
        };
        let out = server
            .read_at_as(
                &caller,
                ReadAtParams {
                    stream_id: None,
                    revision: "working".into(),
                    path: "who.txt".into(),
                },
            )
            .await
            .unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        assert!(text.contains("mine"), "{text}");
    }

    /// tsk574: the extension and lens tools without a `stream_id` read
    /// the calling thread's stream — an agent in a worktree sees the
    /// extension it just wrote there, not the primary's.
    #[tokio::test]
    async fn extension_tools_default_to_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let (_proj, services, server) = boot();
        let primary = services.stream_store.list().await.unwrap().pop().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let ext = elsewhere.path().join("oxplow/extensions/mine");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("extension.yaml"), "name: mine\n").unwrap();
        let mut other = primary.clone();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = services.stream_store.upsert(&other).await.unwrap();
        let thread = new_thread(&services, other_id, "t").await;
        let caller = || {
            extensions_for(parts_with(
                &[("x-oxplow-thread", &thread.id.to_string())],
                "http://h/mcp",
            ))
        };
        let listed = text_payload(
            server
                .list_extensions(caller(), Parameters(StreamScopeParams { stream_id: None }))
                .await
                .unwrap(),
        );
        assert!(listed.contains("\"mine\""), "{listed}");
        // Without a caller, still the primary's.
        let primary_list = text_payload(
            server
                .list_extensions(
                    rmcp::model::Extensions::new(),
                    Parameters(StreamScopeParams { stream_id: None }),
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
        let (_proj, services, server) = boot();
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

    /// tsk862: an effort's observations are its stored evidence rows —
    /// the computation lives in the `effort_evidence` asset, run once.
    #[tokio::test]
    async fn mcp_observations_are_the_stored_rows() {
        use oxplow_db::EffortStore as _;
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let task_id = services
            .task_store
            .insert(&make_task(Some(thread.id), "evidence"))
            .await
            .unwrap();
        let effort = services
            .effort_store
            .start(&work_item_ref(task_id), &thread.id, None)
            .await
            .unwrap();
        let row = |kind: &str, value: f64| oxplow_db::EffortObservation {
            kind: kind.into(),
            provenance: "observed".into(),
            source: "post-tool-bash".into(),
            metric_value: Some(value),
            payload_json: Some("{}".into()),
            local_snapshot_id: None,
            created_at: oxplow_domain::Timestamp::from_unix_ms(1_790_000_000_000),
        };
        services
            .effort_evidence_store
            .replace_observations(
                effort.id.value(),
                vec![row("diff-coverage", 72.5), row("test-run", 1.0)],
            )
            .await
            .unwrap();
        let read = |effort_id: Option<String>, thread_id: Option<String>| {
            server.list_effort_observations(Parameters(ListEffortObservationsParams {
                effort_id,
                thread_id,
                kind: Some("diff-coverage".into()),
            }))
        };
        let by_effort: Vec<oxplow_db::EffortObservation> = serde_json::from_str(&text_payload(
            read(Some(effort.id.to_string()), None).await.unwrap(),
        ))
        .unwrap();
        assert_eq!(by_effort, vec![row("diff-coverage", 72.5)]);
        let by_thread: Vec<oxplow_db::EffortObservation> = serde_json::from_str(&text_payload(
            read(None, Some(thread.id.to_string())).await.unwrap(),
        ))
        .unwrap();
        assert_eq!(by_thread, by_effort);
    }

    #[tokio::test]
    async fn get_open_effort_reports_open_effort() {
        use oxplow_db::EffortStore as _;
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let task_id = services
            .task_store
            .insert(&make_task(Some(thread.id), "open effort task"))
            .await
            .unwrap();
        let effort = services
            .effort_store
            .start(&work_item_ref(task_id), &thread.id, None)
            .await
            .unwrap();

        let r = server
            .get_open_effort(Parameters(GetOpenEffortParams {
                thread_id: thread.id.to_string(),
            }))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(parsed["open"], true);
        assert_eq!(parsed["effortId"], effort.id.to_string());
        assert_eq!(parsed["taskId"], task_id.to_string());
        assert!(parsed["startedAt"].is_string());
        // start() with None records no start snapshot.
        assert_eq!(parsed["hasStartSnapshot"], false);
    }

    #[tokio::test]
    async fn get_open_effort_reports_none_when_no_open_effort() {
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let r = server
            .get_open_effort(Parameters(GetOpenEffortParams {
                thread_id: thread.id.to_string(),
            }))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text_payload(r)).unwrap();
        assert_eq!(parsed["open"], false);
        assert!(parsed["effortId"].is_null());
    }

    #[tokio::test]
    async fn get_open_effort_rejects_non_thread_id() {
        let (_proj, _svc, server) = boot();
        let err = server
            .get_open_effort(Parameters(GetOpenEffortParams {
                thread_id: "str1".into(),
            }))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("thread_id"), "got: {msg}");
    }

    /// Helper to write a wiki body to disk.
    async fn seed_wiki(project: &std::path::Path, slug: &str, body: &str) {
        let wiki_dir = project.join(".oxplow").join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(wiki_dir.join(format!("{slug}.md")), body).unwrap();
    }

    #[tokio::test]
    async fn wiki_ref_drift_reports_status_per_ref() {
        let (proj, svc, server) = boot();
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
        let err =
            expect_id_kind("list_thread_notes", "thread_id", "str123", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(
            msg.contains("list_thread_notes"),
            "tool name missing: {msg}"
        );
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

    // ---- compose_dispatch_brief ----

    #[test]
    fn dispatch_brief_includes_identity_and_protocol() {
        let mut item = make_task(None, "ship the thing");
        item.description = String::new();
        let s = compose_dispatch_brief(&item, "");
        assert!(s.contains("Task: ship the thing"));
        assert!(s.contains(&format!("itemId: {}", item.id.value())));
        assert!(s.contains("priority:"));
        assert!(s.contains("## Protocol"));
        assert!(!s.contains("## Description"));
        assert!(!s.contains("## Extra context"));
    }

    #[test]
    fn dispatch_brief_includes_description_when_non_empty() {
        let mut item = make_task(None, "x");
        item.description = "do the thing carefully".into();
        let s = compose_dispatch_brief(&item, "");
        assert!(s.contains("## Description"));
        assert!(s.contains("do the thing carefully"));
    }

    #[test]
    fn dispatch_brief_appends_extra_context_when_provided() {
        let item = make_task(None, "x");
        let s = compose_dispatch_brief(&item, "see also note n-7");
        assert!(s.contains("## Extra context"));
        assert!(s.contains("see also note n-7"));
    }

    // ---- default_limit ----

    #[test]
    fn default_limit_is_stable() {
        // The exact value is part of the MCP contract; a regression
        // here changes how much data clients receive by default.
        assert_eq!(default_limit(), 20);
    }

    /// Point the in-memory services' `role` at a mock provider at `base`.
    fn assign_mock_role(services: &Services, base: String, role: &str) {
        use oxplow_app::ai_service::{ProviderConfig, ProviderKind, Role, RoleBinding};
        services
            .ai
            .save_provider(
                ProviderConfig {
                    id: "mock".into(),
                    kind: ProviderKind::OpenaiCompatible,
                    base_url: Some(base),
                },
                None,
            )
            .unwrap();
        let role: Role = serde_json::from_value(serde_json::json!(role)).unwrap();
        services
            .ai
            .set_role(
                role,
                Some(RoleBinding {
                    provider: "mock".into(),
                    model: "m".into(),
                }),
            )
            .unwrap();
    }

    #[tokio::test]
    async fn ai_tools_list_roles_decide_and_summarize() {
        let (_proj, services, server) = boot();
        let roles: serde_json::Value =
            serde_json::from_str(&text_payload(server.list_ai_roles().await.unwrap())).unwrap();
        assert_eq!(roles["roles"].as_array().unwrap().len(), 6);
        assert_eq!(roles["roles"][0]["binding"], serde_json::Value::Null);

        // Unassigned role: a clear error, not a crash.
        let err = server
            .ai_summarize(Parameters(AiSummarizeParams {
                text: "t".into(),
                focus: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("summarize"), "{}", err.message);

        let answer = serde_json::json!({"answers": {"risky": {"type": "choice", "choice": "yes", "probabilities": {"yes": 0.7, "no": 0.3}}}});
        let (base, seen) = oxplow_ai::testing::mock(
            "/chat/completions",
            200,
            serde_json::json!({"choices": [{"message": {"content": answer.to_string()}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
        )
        .await;
        assign_mock_role(&services, base.clone(), "decide");
        let out: serde_json::Value = serde_json::from_str(&text_payload(
            server
                .ai_decide(Parameters(AiDecideParams {
                    state: "a diff".into(),
                    questions: std::collections::BTreeMap::from([(
                        "risky".to_string(),
                        AiQuestionParam {
                            kind: "choice".into(),
                            instructions: "Is it risky?".into(),
                            options: Some(vec!["yes".into(), "no".into()]),
                            levels: None,
                        },
                    )]),
                    role: None,
                }))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(out["answers"]["risky"]["choice"], "yes");
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("Is it risky?"));

        let err = server
            .ai_decide(Parameters(AiDecideParams {
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
                role: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("maybe"), "{}", err.message);

        let (base, seen) = oxplow_ai::testing::mock(
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
                    .ai_summarize(Parameters(AiSummarizeParams {
                        text: "long".into(),
                        focus: Some("risks".into()),
                    }))
                    .await
                    .unwrap(),
            );
            assert_eq!(out, "short");
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(seen.lock().unwrap()[0].2["messages"]
            .to_string()
            .contains("risks"));
    }

    #[tokio::test]
    async fn ensure_change_analyzes_a_commit_for_agents() {
        let (proj, _services, server) = boot();
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
