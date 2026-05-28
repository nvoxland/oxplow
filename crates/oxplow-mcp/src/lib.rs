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
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::ref_resolver::{self, RefSummary};
use oxplow_app::{CreateTaskInput, OxplowEvent, Services, UpdateTaskChanges};
use oxplow_domain::comment::CommentThread;
use oxplow_domain::stores::{
    CommentStore, TaskEventStore, TaskLinkStore, TaskNoteStore, TaskStore, ThreadStore,
};
use oxplow_domain::{
    CommentId, CommentStatus, NoteId, StreamId, Task, TaskId, TaskLinkType, TaskStatus, ThreadId,
};

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

// ---------- request shapes ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StreamIdParams {
    pub stream_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ThreadIdParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TaskIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReorderTasksParams {
    /// Optional thread scope. Omit for the project-wide backlog.
    pub thread_id: Option<String>,
    /// New sort order. Items not present keep their relative order
    /// at the end of the list.
    pub ordered_item_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DelegateQueryParams {
    pub thread_id: String,
    pub question: String,
    pub focus: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RecordQueryFindingParams {
    pub note_id: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpsertTaskParams {
    /// JSON-encoded Task. Use this rather than nesting the struct
    /// directly so we don't have to plumb JsonSchema through every
    /// domain type.
    pub item_json: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestCoverageParams {
    pub thread_id: String,
    /// Repo-relative path to the coverage report. Omit to use the
    /// project's configured `collection.coverageReportPath`.
    pub report_path: Option<String>,
    /// `cobertura` | `lcov` | `jacoco-xml`. Omit to use the configured
    /// `collection.coverageFormat`.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RecordTestRunParams {
    pub thread_id: String,
    pub command: String,
    pub passed: Option<i64>,
    pub failed: Option<i64>,
    pub total: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListEffortObservationsParams {
    /// Effort id to read directly. Omit to use the open effort on
    /// `thread_id`.
    pub effort_id: Option<String>,
    pub thread_id: Option<String>,
    /// Optional kind filter: `test-run` | `diff-coverage`.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddThreadNoteParams {
    pub thread_id: String,
    pub body: String,
    pub author: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeleteNoteParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListCommentsParams {
    /// `"thread"` (id = `b-…`) or `"stream"` (id = `s-…`).
    pub scope: String,
    /// The thread or stream id matching `scope`.
    pub id: String,
    /// Filter: `"all"` (default), `"open"`, or `"needs_response"`.
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RespondToCommentParams {
    /// Integer comment id (from `list_comments`).
    pub comment_id: i64,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CommentIdParams {
    /// Integer comment id (from `list_comments`).
    pub comment_id: i64,
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
pub struct SubsystemDocParams {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateTaskMcpParams {
    /// Thread to attach the new item to. Required unless `backlog`
    /// is set to `true` — filing onto the project-wide backlog must
    /// be an explicit choice, since a thread-detached row trips
    /// filing-enforcement on the next edit.
    pub thread_id: Option<String>,
    /// Set to `true` to file the item onto the project-wide backlog
    /// (no thread attachment). Mutually exclusive with `thread_id`.
    /// Default `false`: a missing `thread_id` is an error.
    #[serde(default)]
    pub backlog: bool,
    pub title: String,
    /// Markdown body. If acceptance criteria would help reviewers (or
    /// future-you) know when this task is done, write them inline here
    /// as a `## Acceptance criteria` subsection. There is no separate
    /// AC field — the description is the single source of truth.
    pub description: Option<String>,
    /// Optional executive-summary rewrite of `description` (shorter,
    /// overview-level). Keep its heading skeleton aligned with the
    /// developer text so section-anchored comments resolve across
    /// audience variants.
    pub description_executive: Option<String>,
    /// Optional terse "caveman"-style rewrite of `description` (drop
    /// filler, sentence fragments, keep technical terms / paths / code
    /// verbatim). Keep its heading skeleton aligned with the developer
    /// text.
    pub description_caveman: Option<String>,
    pub kind: Option<String>,
    pub priority: Option<String>,
    pub parent_id: Option<String>,
    /// Initial status — defaults to `ready`. Pass `in_progress`
    /// when starting the work in the same call (filing-enforcement
    /// requires an in_progress row to exist before edits land), or
    /// `done`/`blocked` when filing a row for already-shipped work
    /// (`touched_files` then drives Local History attribution).
    pub status: Option<String>,
    /// Repo-relative paths edited for this effort. When passed
    /// alongside `status: "done"` or `"blocked"`, the runtime
    /// synthesizes the in_progress→target effort transition so
    /// Local History attributes the writes to this item.
    pub touched_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateTaskMcpParams {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Optional executive-summary rewrite of `description`. Present →
    /// replace, missing → keep the existing variant.
    pub description_executive: Option<String>,
    /// Optional terse "caveman"-style rewrite of `description`.
    /// Present → replace, missing → keep.
    pub description_caveman: Option<String>,
    /// Reparent (or detach with empty string).
    pub parent_id: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    /// Repo-relative paths edited for the effort that's closing
    /// alongside this update. Required for Local History attribution
    /// when transitioning to `done`/`blocked` from `in_progress`.
    pub touched_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct McpTaskImpact {
    /// Page kind: `wiki | task | file | directory | git_commit |
    /// finding`. Snake-case on the wire; normalized at projection.
    pub kind: String,
    /// Canonical id for that page kind (wiki slug, integer task id
    /// as string, repo-relative file/directory path, commit sha,
    /// finding id).
    pub id: String,
    /// What happened to it: `created | updated | deleted |
    /// referenced | resolved | completed | reopened`. Free-form;
    /// renders as a chip in the UI when present.
    pub action: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CompleteTaskParams {
    pub id: String,
    /// Summary note appended to the task before marking done
    /// (developer audience — the canonical text).
    pub summary: String,
    /// Optional executive-summary rewrite of `summary`.
    pub summary_executive: Option<String>,
    /// Optional terse "caveman"-style rewrite of `summary`.
    pub summary_caveman: Option<String>,
    pub author: Option<String>,
    /// Repo-relative paths edited for this effort. Drives the file-
    /// attribution effort row Local History reads from.
    pub touched_files: Option<Vec<String>>,
    /// Cross-page outcomes the LLM declares — wiki pages created
    /// or updated, tasks completed/reopened, commits referenced,
    /// findings resolved, etc. Each is projected into the
    /// `page_ref` graph as an outbound edge from this task, so
    /// backlinks on the target page show this task as the cause
    /// without relying on summary-body parsing.
    pub impacts: Option<Vec<McpTaskImpact>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AmendEffortParams {
    /// Effort id (the `id` returned on `task_effort` rows). Find it
    /// via `get_task` → `efforts[].id` or by inspecting the
    /// reconciliation payload returned from `complete_task`.
    pub effort_id: String,
    /// Repo-relative paths to ADD to the effort's touched_files
    /// list. Use these to claim files the auto-diff missed.
    pub add_files: Option<Vec<String>>,
    /// Repo-relative paths to REMOVE from the effort's touched_files
    /// list. Use these to disclaim files the auto-diff thought were
    /// yours but actually came from another actor (formatter, parallel
    /// effort, the user, etc.).
    pub remove_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LinktasksParams {
    pub thread_id: String,
    pub from_id: String,
    pub to_id: String,
    /// One of: blocks, relates_to, discovered_from, duplicates,
    /// supersedes, replies_to.
    pub link_type: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TransitiontasksParams {
    pub ids: Vec<String>,
    pub status: String,
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
pub struct FileEpicWithChildrenParams {
    pub thread_id: Option<String>,
    pub epic_title: String,
    pub epic_description: Option<String>,
    /// Optional executive-summary rewrite of `epic_description`.
    pub epic_description_executive: Option<String>,
    /// Optional terse "caveman"-style rewrite of `epic_description`.
    pub epic_description_caveman: Option<String>,
    pub children: Vec<EpicChildSpec>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EpicChildSpec {
    pub title: String,
    pub description: Option<String>,
    /// Optional executive-summary rewrite of `description`.
    pub description_executive: Option<String>,
    /// Optional terse "caveman"-style rewrite of `description`.
    pub description_caveman: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DispatchTaskParams {
    pub thread_id: String,
    /// The specific task to dispatch. When omitted, picks the
    /// first ready item on the thread (mirrors main's
    /// dispatch-without-id shortcut for /work-next composition).
    pub item_id: Option<String>,
    /// Optional extra context appended to the brief — usually
    /// orchestrator notes about how this fits into the larger plan.
    pub extra_context: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ForkThreadParams {
    pub source_thread_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FindNotesForNoteParams {
    pub slug: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PageRefParams {
    /// Page kind, e.g. "wiki", "task", "file", "git-commit",
    /// "finding", "directory".
    pub kind: String,
    /// Canonical page id within the kind. For files this is the
    /// repo-relative path; for tasks the `wi-…` id; for
    /// commits the full sha; for wiki pages the slug.
    pub id: String,
    #[serde(default = "default_page_ref_limit")]
    pub limit: u32,
}

fn default_page_ref_limit() -> u32 {
    100
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ResyncNoteParams {
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RecordWikiPageUpdateParams {
    /// Wiki slug (without `.md`).
    pub slug: String,
    /// Repo-relative paths the agent re-read against the new body
    /// during this edit. Each MUST appear as a `[[…]]` or
    /// `[label](file:…)` reference in the new body, or the call
    /// errors. Pass `[]` to declare "I didn't re-check any refs
    /// this turn" — empty is allowed but the field is required so
    /// the agent can't sleepwalk past freshness bookkeeping.
    pub verified_refs: Vec<String>,
    /// Repo-relative paths the agent intentionally removed from
    /// the page in this edit. Each MUST NOT appear in the new
    /// body. Pass `[]` if no refs were removed.
    pub removed_refs: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LspPositionParams {
    pub stream_id: String,
    pub language: String,
    pub uri: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LspDiagnosticsParams {
    pub stream_id: String,
    pub language: String,
    pub uri: String,
}

/// Optional stream selector shared by the stream-scoped git read tools.
/// Omit `stream_id` to target the current/primary worktree.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct GitStreamParams {
    pub stream_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitLogParams {
    pub stream_id: Option<String>,
    /// Max commits to return.
    pub limit: Option<u32>,
    /// Include all branches (`--all`) rather than just the current branch.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitPathParams {
    pub stream_id: Option<String>,
    /// Repo-relative file path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitDiffParams {
    pub stream_id: Option<String>,
    /// Base ref to diff the branch against (e.g. `main`).
    pub base_ref: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GitReadAtRefParams {
    /// Git ref (branch, tag, or sha) to read the file at.
    pub git_ref: String,
    /// Repo-relative file path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SnapshotStreamParams {
    pub stream_id: String,
    /// Max snapshots to return (default 200).
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
    /// A `snapshot` or `file_snapshot` row id (integer).
    pub snapshot_id: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunCodeQualityScanParams {
    /// Analysis kind: `"metrics"` (per-function complexity/length/params) or
    /// `"duplication"` (duplicate-block detection).
    pub tool: String,
    /// Free-form scope label (typically `"workspace"` or `"diff"`).
    pub scope: String,
    /// Optional subset of repo-relative paths; omit to scan the whole repo.
    pub files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeQualityScanIdParams {
    /// Scan id returned by `run_code_quality_scan` / `list_code_quality_scans`.
    pub scan_id: i64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CodeQualityListParams {
    /// Max scans to return (default 20).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateCommentMcpParams {
    pub stream_id: String,
    /// Optional thread to attribute the comment to.
    pub thread_id: Option<String>,
    /// Page-kind scheme of the target: `wiki | file | directory | task | \
    /// git-commit | finding`.
    pub target_kind: String,
    /// Canonical id for that kind (wiki slug, repo-relative path, task id, …).
    pub target_id: String,
    pub body: String,
    /// Optional quoted span the comment is about (empty = whole-target note).
    pub quote: Option<String>,
    /// `note` (default) or `followup`.
    pub intent: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetCommentIntentParams {
    /// Integer comment id from `list_comments`.
    pub comment_id: i64,
    /// `note` or `followup`.
    pub intent: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RenameThreadMcpParams {
    pub thread_id: String,
    pub title: String,
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
pub struct RenameStreamParams {
    pub stream_id: String,
    pub title: String,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    /// Emit `TasksChanged` so the renderer (which is a separate
    /// process from the MCP server) refetches and reflects the
    /// mutation. The Tauri command layer emits its own events; MCP
    /// has to do the same or UI state silently goes stale after every
    /// agent-driven change.
    fn emit_tasks_changed(&self, thread_id: Option<oxplow_domain::ThreadId>) {
        self.services
            .events
            .emit(OxplowEvent::TasksChanged { thread_id });
    }

    /// Emit `ThreadsChanged` so the renderer (separate process) refetches a
    /// stream's threads after an agent-driven lifecycle change.
    fn emit_threads_changed(&self, stream_id: oxplow_domain::StreamId) {
        self.services
            .events
            .emit(OxplowEvent::ThreadsChanged { stream_id });
    }

    /// Renderer is a separate process; emit so it refetches the page's
    /// comments + the Comments inbox after an agent-driven change.
    fn emit_comments_changed(&self, comment: &oxplow_domain::Comment) {
        self.services.events.emit(OxplowEvent::CommentsChanged {
            stream_id: comment.stream_id.clone(),
            target_kind: comment.target_kind.clone(),
            target_id: comment.target_id.clone(),
        });
    }

    // ---------- liveness / version ----------

    #[tool(description = "Liveness check: returns \"pong\".")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    #[tool(description = "Get the running oxplow daemon version.")]
    async fn app_version(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(env!(
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
    // Thin mirrors of the IPC git read commands over `services.git`, so the
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
        params: Parameters<GitStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("git_status", params.0.stream_id.as_deref())?;
        let scopes = self
            .services
            .git
            .change_scopes(params.0.stream_id.as_deref())
            .await;
        json_result(&scopes)
    }

    #[tool(
        description = "Git commit log for the worktree. `all` spans every branch; \
                          `limit` caps the count."
    )]
    async fn git_log(&self, params: Parameters<GitLogParams>) -> Result<CallToolResult, McpError> {
        check_optional_stream("git_log", params.0.stream_id.as_deref())?;
        let opts = oxplow_git::GitLogOptions {
            limit: params.0.limit.map(|n| n as usize),
            all: params.0.all,
        };
        let log = self
            .services
            .git
            .git_log(params.0.stream_id.as_deref(), opts)
            .await;
        json_result(&log)
    }

    #[tool(description = "Git blame for a file: per-line commit attribution.")]
    async fn git_blame(
        &self,
        params: Parameters<GitPathParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("git_blame", params.0.stream_id.as_deref())?;
        let lines = self
            .services
            .git
            .blame(params.0.stream_id.as_deref(), params.0.path)
            .await;
        json_result(&lines)
    }

    #[tool(
        description = "Git branch diff: per-file and per-function changes on the \
                          worktree branch relative to `base_ref` (e.g. `main`)."
    )]
    async fn git_diff(
        &self,
        params: Parameters<GitDiffParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("git_diff", params.0.stream_id.as_deref())?;
        let changes = self
            .services
            .git
            .branch_changes(params.0.stream_id.as_deref(), params.0.base_ref)
            .await;
        json_result(&changes)
    }

    #[tool(
        description = "Read a file's contents at a git ref (branch, tag, or sha). \
                          Returns null when the path doesn't exist at that ref."
    )]
    async fn read_file_at_ref(
        &self,
        params: Parameters<GitReadAtRefParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = self
            .services
            .git
            .read_file_at_ref(params.0.git_ref, params.0.path)
            .await;
        json_result(&content)
    }

    #[tool(description = "List the project's git branches (local + remote).")]
    async fn list_branches(&self) -> Result<CallToolResult, McpError> {
        let branches = self.services.git.list_branches_project().await;
        json_result(&branches)
    }

    // ---------- snapshots / local history (read + restore) ----------
    //
    // Thin mirrors of the IPC snapshot reads over `services.snapshot_store` /
    // `blobs`, so the agent can inspect and restore its own change history.
    // (Unlike the UI reads, these don't strip `generated` paths — the agent
    // sees the raw capture history.) The composed dashboard DTOs
    // (`get_snapshot_summary`, `get_snapshot_pair_diff`) stay IPC-only; their
    // logic lives in the command layer, so the agent composes equivalents from
    // `get_snapshot` + `read_snapshot_file_content`.

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
        let rows = self
            .services
            .snapshot_store
            .list_snapshots_for_stream(&params.0.stream_id, params.0.limit.unwrap_or(200) as usize)
            .await
            .map_err(internal)?;
        json_result(&rows)
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

    #[tool(description = "Get a single file_snapshot row by id (null if absent).")]
    async fn get_snapshot(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let row = self
            .services
            .snapshot_store
            .get(params.0.snapshot_id)
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
        description = "Read a file_snapshot's blob content as a (UTF-8 lossy) string. \
                          Null when the row, blob, or content is absent."
    )]
    async fn read_snapshot_file_content(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = match self
            .services
            .snapshot_store
            .get(params.0.snapshot_id)
            .await
            .map_err(internal)?
            .and_then(|snap| snap.blob_hash)
        {
            Some(hash) => match self.services.blobs.read(&hash) {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Err(_) => None,
            },
            None => None,
        };
        json_result(&content)
    }

    #[tool(
        description = "Restore a file's contents from a snapshot, writing the blob back \
                          to its workspace path. Errors if the row or blob is gone."
    )]
    async fn restore_file_from_snapshot(
        &self,
        params: Parameters<SnapshotIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let snap = self
            .services
            .snapshot_store
            .get(params.0.snapshot_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| McpError::invalid_params("snapshot row not found", None))?;
        let hash = snap.blob_hash.clone().ok_or_else(|| {
            McpError::invalid_params("snapshot has no blob (oversize or pre-blob-store)", None)
        })?;
        let bytes = self.services.blobs.read(&hash).map_err(internal)?;
        let target = self.services.layout.project_dir.join(&snap.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(internal)?;
        }
        std::fs::write(&target, &bytes).map_err(internal)?;
        json_result(&serde_json::json!({ "restored": snap.path }))
    }

    // ---------- code quality ----------
    //
    // Run a scan and read its findings, so the agent can close the loop on its
    // own changes. `run_code_quality_scan` shares the IPC command's
    // orchestration via `Services::run_code_quality_scan`.

    #[tool(
        description = "Run a code-quality scan and persist findings; returns the scan id. \
                          `tool` is \"metrics\" or \"duplication\"; `scope` is a free-form \
                          label (e.g. \"workspace\"); `files` optionally narrows the scan."
    )]
    async fn run_code_quality_scan(
        &self,
        params: Parameters<RunCodeQualityScanParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let scan_id = self
            .services
            .run_code_quality_scan(p.tool, p.scope, p.files)
            .await
            .map_err(|e| match e {
                oxplow_app::code_quality_runner::CodeQualityError::UnknownTool(_) => {
                    McpError::invalid_params(e.to_string(), None)
                }
                other => internal(other),
            })?;
        json_result(&serde_json::json!({ "scan_id": scan_id }))
    }

    #[tool(description = "List recent code-quality scans (newest first).")]
    async fn list_code_quality_scans(
        &self,
        params: Parameters<CodeQualityListParams>,
    ) -> Result<CallToolResult, McpError> {
        let scans = self
            .services
            .code_quality_store
            .list_scans(params.0.limit.unwrap_or(20) as usize)
            .await
            .map_err(internal)?;
        json_result(&scans)
    }

    #[tool(description = "List the findings produced by a code-quality scan.")]
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

    // ---------- comments + stream/thread lifecycle ----------
    //
    // Originate comments and manage thread/stream lifecycle, matching the UI's
    // affordances over the same services. Each mutation emits the same event
    // the IPC command does, so the (separate-process) renderer refetches.
    // Stream-branch checkout stays on Bash (subprocess logic lives in the IPC
    // command layer, and the agent's worktree shell can `git checkout`).

    #[tool(
        description = "Create a comment anchored to a target (wiki/file/task/…). Omit `quote` \
                          for a whole-target note. `intent` is `note` (default) or `followup`."
    )]
    async fn create_comment(
        &self,
        params: Parameters<CreateCommentMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("create_comment", "stream_id", &p.stream_id, ID_STREAM)?;
        if let Some(tid) = &p.thread_id {
            expect_id_kind("create_comment", "thread_id", tid, ID_THREAD)?;
        }
        let intent = parse_comment_intent("create_comment", p.intent.as_deref().unwrap_or("note"))?;
        let stream_id = StreamId::from(p.stream_id);
        let thread_id = p.thread_id.map(ThreadId::from);
        let target = oxplow_domain::CommentTarget {
            kind: p.target_kind,
            id: p.target_id,
        };
        let thread = self
            .services
            .comment_store
            .create(
                &stream_id,
                thread_id.as_ref(),
                &target,
                p.quote.as_deref().unwrap_or(""),
                "",
                &[],
                &[],
                intent,
                "agent",
                &p.body,
            )
            .await
            .map_err(internal)?;
        self.emit_comments_changed(&thread.comment);
        json_result(&thread)
    }

    #[tool(
        description = "Set a comment's intent: `note` (agent leaves it alone) or `followup` \
                          (agent should act on it)."
    )]
    async fn set_comment_intent(
        &self,
        params: Parameters<SetCommentIntentParams>,
    ) -> Result<CallToolResult, McpError> {
        let intent = parse_comment_intent("set_comment_intent", &params.0.intent)?;
        let id = CommentId::new(params.0.comment_id);
        self.services
            .comment_store
            .set_intent(id, intent)
            .await
            .map_err(internal)?;
        let thread = self
            .services
            .comment_store
            .get(id)
            .await
            .map_err(internal)?;
        if let Some(t) = &thread {
            self.emit_comments_changed(&t.comment);
        }
        json_result(&thread)
    }

    #[tool(description = "Rename a thread.")]
    async fn rename_thread(
        &self,
        params: Parameters<RenameThreadMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("rename_thread", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let id = ThreadId::from(params.0.thread_id);
        let thread = self
            .services
            .threads
            .rename(&id, params.0.title)
            .await
            .map_err(internal)?;
        self.emit_threads_changed(thread.stream_id.clone());
        json_result(&thread)
    }

    #[tool(description = "Promote a thread to the top of its stream's working queue.")]
    async fn promote_thread(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "promote_thread",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let thread = self.services.threads.promote(&id).await.map_err(internal)?;
        self.emit_threads_changed(thread.stream_id.clone());
        json_result(&thread)
    }

    #[tool(description = "Close a thread (soft — reopenable).")]
    async fn close_thread(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("close_thread", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let id = ThreadId::from(params.0.thread_id);
        let thread = self.services.threads.close(&id).await.map_err(internal)?;
        self.emit_threads_changed(thread.stream_id.clone());
        json_result(&thread)
    }

    #[tool(description = "Reopen a closed thread.")]
    async fn reopen_thread(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("reopen_thread", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let id = ThreadId::from(params.0.thread_id);
        let thread = self.services.threads.reopen(&id).await.map_err(internal)?;
        self.emit_threads_changed(thread.stream_id.clone());
        json_result(&thread)
    }

    #[tool(description = "Select (focus) a thread on a stream, or clear the selection.")]
    async fn select_thread(
        &self,
        params: Parameters<SelectThreadMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("select_thread", "stream_id", &params.0.stream_id, ID_STREAM)?;
        if let Some(tid) = &params.0.thread_id {
            expect_id_kind("select_thread", "thread_id", tid, ID_THREAD)?;
        }
        let stream_id = StreamId::from(params.0.stream_id);
        let thread_id = params.0.thread_id.map(ThreadId::from);
        self.services
            .threads
            .select(&stream_id, thread_id.as_ref())
            .await
            .map_err(internal)?;
        self.services
            .events
            .emit(OxplowEvent::SelectedThreadChanged {
                stream_id,
                thread_id,
            });
        json_result(&serde_json::json!({ "ok": true }))
    }

    #[tool(description = "Set the current/active stream (or omit stream_id to clear it).")]
    async fn switch_stream(
        &self,
        params: Parameters<SwitchStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        check_optional_stream("switch_stream", params.0.stream_id.as_deref())?;
        let id = params.0.stream_id.map(StreamId::from);
        self.services
            .streams
            .set_current(id.as_ref())
            .await
            .map_err(internal)?;
        self.services
            .events
            .emit(OxplowEvent::CurrentStreamChanged { stream_id: id });
        json_result(&serde_json::json!({ "ok": true }))
    }

    #[tool(description = "Rename a stream.")]
    async fn rename_stream(
        &self,
        params: Parameters<RenameStreamParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("rename_stream", "stream_id", &params.0.stream_id, ID_STREAM)?;
        let id = StreamId::from(params.0.stream_id);
        let stream = self
            .services
            .streams
            .rename(&id, params.0.title)
            .await
            .map_err(internal)?;
        self.services.events.emit(OxplowEvent::StreamsChanged);
        json_result(&stream)
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
        let stream_id = oxplow_domain::StreamId::from(params.0.stream_id);
        let list = self
            .services
            .thread_store
            .list_for_stream(&stream_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    // ---------- tasks ----------

    #[tool(description = "List all tasks on the project-wide backlog.")]
    async fn list_backlog(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .task_store
            .list_backlog()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(description = "List tasks on a thread.")]
    async fn list_ready_work(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_ready_work",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = ThreadId::from(params.0.thread_id);
        let list = self
            .services
            .task_store
            .list_for_thread(&thread_id)
            .await
            .map_err(internal)?;
        json_result(&list)
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
        let thread_id = ThreadId::from(params.0.thread_id);
        let result = self
            .services
            .tasks
            .read_task_options(&thread_id, &*self.services.task_link_store)
            .await
            .map_err(internal)?;
        json_result(&result)
    }

    #[tool(
        description = "Reorder tasks on a thread (or backlog). The orderedItemIds array becomes \
                       the new sort order; items not in the list keep their relative order at the end."
    )]
    async fn reorder_tasks(
        &self,
        params: Parameters<ReorderTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(t) = params.0.thread_id.as_deref() {
            expect_id_kind("reorder_tasks", "thread_id", t, ID_THREAD)?;
        }
        let mut ids: Vec<TaskId> = Vec::with_capacity(params.0.ordered_item_ids.len());
        for raw in &params.0.ordered_item_ids {
            ids.push(parse_task_id("reorder_tasks", "ordered_item_ids[]", raw)?);
        }
        let thread = params
            .0
            .thread_id
            .as_deref()
            .map(|s| ThreadId::from(s.to_string()));
        self.services
            .tasks
            .reorder(thread.as_ref(), &ids)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(thread);
        json_result(&serde_json::json!({ "ok": true }))
    }

    #[tool(description = "Get a single task by id.")]
    async fn get_task(&self, params: Parameters<TaskIdParams>) -> Result<CallToolResult, McpError> {
        let id = parse_task_id("get_task", "id", &params.0.id)?;
        let item = self.services.task_store.get(id).await.map_err(internal)?;
        json_result(&item)
    }

    #[tool(
        description = "Persist (insert or update) a task. `item_json` is the JSON-encoded Task."
    )]
    async fn upsert_task(
        &self,
        params: Parameters<UpsertTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut item: Task = serde_json::from_str(&params.0.item_json)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        if item.id.value() == 0 {
            let new_id = self
                .services
                .task_store
                .insert(&item)
                .await
                .map_err(internal)?;
            item.id = new_id;
        } else {
            self.services
                .task_store
                .update(&item)
                .await
                .map_err(internal)?;
        }
        self.emit_tasks_changed(item.thread_id.clone());
        json_result(&item)
    }

    #[tool(description = "Soft-delete a task by id.")]
    async fn delete_task(
        &self,
        params: Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_task_id("delete_task", "id", &params.0.id)?;
        let item = self.services.task_store.get(id).await.map_err(internal)?;
        self.services
            .task_store
            .soft_delete(id)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(item.and_then(|i| i.thread_id));
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- thread notes ----------
    //
    // Per-task notes (`add_work_note` / `list_work_notes`) were
    // retired: `task_effort.summary` already carries "what
    // shipped on this item", so a parallel note table for the same
    // purpose was duplicative. Thread-scoped notes stay — they back
    // the Explore-subagent findings flow.

    #[tool(description = "Add a thread-scoped note (not attached to any item).")]
    async fn add_thread_note(
        &self,
        params: Parameters<AddThreadNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "add_thread_note",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let note = self
            .services
            .work_note_store
            .add_for_thread(&id, &params.0.body, &params.0.author)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

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
        let id = ThreadId::from(params.0.thread_id);
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
        description = "Ingest a coverage report into the thread's open effort as diff coverage \
            over the lines that effort changed. oxplow parses the report deterministically \
            (cobertura / lcov / jacoco-xml) — you only point at it, you NEVER report coverage \
            numbers yourself (that keeps the result trustworthy/`observed`). `report_path` and \
            `format` default to the project's `collection` profile in oxplow.yaml. Returns a \
            status: stored (with summaryPct over changed lines) or a reason nothing landed \
            (no_open_effort / not_configured / report_missing / no_baseline / no_changed_coverage)."
    )]
    async fn ingest_coverage(
        &self,
        params: Parameters<IngestCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "ingest_coverage",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let tid = ThreadId::from(params.0.thread_id);
        let outcome = self
            .services
            .collection
            .ingest_coverage(&tid, params.0.report_path, params.0.format, false)
            .await
            .map_err(internal)?;
        json_result(&ingest_outcome_json(&outcome))
    }

    #[tool(
        description = "Record a test run against the thread's open effort with pass/fail counts \
            the Bash-hook exit code can't capture. Marked `asserted` (agent-reported). oxplow \
            already records `observed` test runs automatically from the Bash hook — use this \
            only when you want to attach structured pass/fail/total counts."
    )]
    async fn record_test_run(
        &self,
        params: Parameters<RecordTestRunParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "record_test_run",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let tid = ThreadId::from(params.0.thread_id);
        let id = self
            .services
            .collection
            .record_test_run(
                &tid,
                &params.0.command,
                None,
                params.0.duration_ms,
                params.0.passed,
                params.0.failed,
                params.0.total,
                "asserted",
                "agent",
                None,
            )
            .await
            .map_err(internal)?;
        json_result(&serde_json::json!({
            "recorded": id.is_some(),
            "observationId": id,
        }))
    }

    #[tool(
        description = "List collection observations (test-run / diff-coverage) for an effort. \
            Pass `effort_id` to read it directly, or `thread_id` to use that thread's open \
            effort. Optional `kind` filter."
    )]
    async fn list_effort_observations(
        &self,
        params: Parameters<ListEffortObservationsParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_db::TaskEffortStore as _;
        let effort_id = match (params.0.effort_id, params.0.thread_id) {
            (Some(e), _) => e,
            (None, Some(t)) => {
                expect_id_kind("list_effort_observations", "thread_id", &t, ID_THREAD)?;
                let tid = ThreadId::from(t);
                match self
                    .services
                    .effort_store
                    .find_open_for_thread(&tid)
                    .await
                    .map_err(internal)?
                {
                    Some(effort) => effort.id.as_str().to_string(),
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
        let rows = self
            .services
            .collection
            .list_for_effort(&effort_id, params.0.kind.as_deref())
            .await
            .map_err(internal)?;
        json_result(&rows)
    }

    // ---------- comments ----------

    #[tool(
        description = "List comments — threaded annotations the user anchored to a text selection \
                       in a page (wiki body, code file line, task detail). `scope` is \"thread\" \
                       (id = b-…) or \"stream\" (id = s-…, every page in the workspace). `status` \
                       filters: \"all\" (default), \"open\", or \"needs_response\" (open follow-ups \
                       whose latest message isn't yours — i.e. what the user wants you to act on). \
                       Each result carries the anchored `quote`, the message thread, and `intent` \
                       (note vs followup). Respond with respond_to_comment; close with resolve_comment."
    )]
    async fn list_comments(
        &self,
        params: Parameters<ListCommentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let threads = match params.0.scope.as_str() {
            "thread" => {
                expect_id_kind("list_comments", "id", &params.0.id, ID_THREAD)?;
                let id = ThreadId::from(params.0.id);
                self.services
                    .comment_store
                    .list_for_thread(&id)
                    .await
                    .map_err(internal)?
            }
            "stream" => {
                expect_id_kind("list_comments", "id", &params.0.id, ID_STREAM)?;
                let id = StreamId::from(params.0.id);
                self.services
                    .comment_store
                    .list_for_stream(&id)
                    .await
                    .map_err(internal)?
            }
            other => {
                return Err(McpError::invalid_params(
                    format!(
                        "list_comments: `scope` must be \"thread\" or \"stream\", got `{other}`"
                    ),
                    None,
                ));
            }
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

    #[tool(
        description = "Respond to a comment: append your reply to its thread (recorded as author \
                       \"agent\"). This marks an open follow-up answered until the user replies \
                       again. `comment_id` is the integer id from list_comments. Returns the \
                       updated thread."
    )]
    async fn respond_to_comment(
        &self,
        params: Parameters<RespondToCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = CommentId::new(params.0.comment_id);
        self.services
            .comment_store
            .add_message(id, "agent", &params.0.body)
            .await
            .map_err(internal)?;
        let thread = self
            .services
            .comment_store
            .get(id)
            .await
            .map_err(internal)?;
        if let Some(t) = &thread {
            self.emit_comments_changed(&t.comment);
        }
        json_result(&thread)
    }

    #[tool(
        description = "Resolve a comment thread (status = resolved). Use when the user's note is \
                       fully addressed. `comment_id` is the integer id from list_comments."
    )]
    async fn resolve_comment(
        &self,
        params: Parameters<CommentIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = CommentId::new(params.0.comment_id);
        self.services
            .comment_store
            .set_status(id, CommentStatus::Resolved)
            .await
            .map_err(internal)?;
        let thread = self
            .services
            .comment_store
            .get(id)
            .await
            .map_err(internal)?;
        if let Some(t) = &thread {
            self.emit_comments_changed(&t.comment);
        }
        json_result(&thread)
    }

    #[tool(
        description = "Prepare an exploration query for an Explore subagent. Use when you need to \
                       understand a codebase area before dispatching real work and would otherwise \
                       read 5+ files inline — offloading the reads keeps your own cached context \
                       small. Returns { prompt, provisionalNoteId }. The orchestrator then calls \
                       Agent(subagent_type='Explore', prompt=<prompt>); the prompt instructs the \
                       subagent to call mcp__oxplow__record_query_finding({ noteId: \
                       <provisionalNoteId>, body }) with its findings. Read the finding later via \
                       mcp__oxplow__list_thread_notes."
    )]
    async fn delegate_query(
        &self,
        params: Parameters<DelegateQueryParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "delegate_query",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = ThreadId::from(params.0.thread_id.clone());
        let question = params.0.question.trim().to_string();
        if question.is_empty() {
            return Err(McpError::invalid_params(
                "delegate_query: `question` is required",
                None,
            ));
        }
        let focus = params.0.focus.unwrap_or_default().trim().to_string();
        // Allocate the finding note up front with an empty body. The
        // subagent fills it in via record_query_finding when done.
        let provisional = self
            .services
            .work_note_store
            .add_for_thread(&thread_id, "", "explore-subagent")
            .await
            .map_err(internal)?;
        let prompt = compose_delegate_query_prompt(
            &params.0.thread_id,
            &question,
            &focus,
            provisional.id.as_str(),
        );
        json_result(&serde_json::json!({
            "ok": true,
            "prompt": prompt,
            "provisionalNoteId": provisional.id.as_str(),
        }))
    }

    #[tool(
        description = "Write the Explore subagent's finding into a pre-allocated thread-scoped note \
                       (id returned by mcp__oxplow__delegate_query). Call this once at the end of \
                       the exploration — the orchestrator reads it later via list_thread_notes."
    )]
    async fn record_query_finding(
        &self,
        params: Parameters<RecordQueryFindingParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.0.note_id.is_empty() {
            return Err(McpError::invalid_params(
                "record_query_finding: `noteId` is required",
                None,
            ));
        }
        expect_id_kind(
            "record_query_finding",
            "note_id",
            &params.0.note_id,
            ID_NOTE,
        )?;
        let id = NoteId::from(params.0.note_id.clone());
        self.services
            .work_note_store
            .update_body(&id, &params.0.body)
            .await
            .map_err(internal)?;
        json_result(&serde_json::json!({ "ok": true, "noteId": params.0.note_id }))
    }

    #[tool(description = "Delete a note by id.")]
    async fn delete_wiki_page(
        &self,
        params: Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("delete_wiki_page", "id", &params.0.id, ID_NOTE)?;
        let id = NoteId::from(params.0.id);
        self.services
            .work_note_store
            .delete(&id)
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- wiki pages ----------

    #[tool(description = "List all wiki pages (metadata only).")]
    async fn list_wiki_pages(&self) -> Result<CallToolResult, McpError> {
        let notes = self
            .services
            .wiki_page_store
            .list()
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(description = "Title/slug glob search over wiki pages.")]
    async fn search_wiki_pages(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_page_store
            .search_titles(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "FTS5-backed body search over wiki pages; returns ranked snippets.")]
    async fn search_wiki_page_bodies(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_page_store
            .search_bodies(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "Get one wiki page's metadata by slug, enriched with \
                       `stale_refs` — the file refs whose pinned snapshot \
                       is older than the file's latest snapshot. That \
                       freshness detail is the one thing `list_wiki_pages` \
                       does NOT carry; for the bulk fields (title, refs, \
                       excerpt, timestamps) `list_wiki_pages` already \
                       returns everything this does, so don't follow a \
                       `list` with per-page `get` calls unless you need \
                       `stale_refs`. Returns null for an unknown slug.")]
    async fn get_wiki_page_metadata(
        &self,
        params: Parameters<SlugParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(note) = self
            .services
            .wiki_page_store
            .get(&params.0.slug)
            .await
            .map_err(internal)?
        else {
            return json_result(&serde_json::Value::Null);
        };
        let stale_refs: Vec<String> = self
            .services
            .page_ref_store
            .list_wiki_file_freshness(&params.0.slug)
            .await
            .map_err(internal)?
            .into_iter()
            .filter(|(_path, local, _git, _exact, latest)| wiki_ref_stale(*local, *latest))
            .map(|(path, ..)| path)
            .collect();
        let mut value = serde_json::to_value(&note).map_err(internal)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert("stale_refs".to_string(), serde_json::json!(stale_refs));
        }
        json_result(&value)
    }

    #[tool(description = "List wiki pages that have at least one STALE file \
                       reference — a referenced file has been snapshotted \
                       more recently than the page's captured pin (or the \
                       ref was never pinned). Use this to find out-of-date \
                       pages without reading any body. Returns \
                       `{ slug, title, stale_refs }` per drifted page; an \
                       empty array means every page is current.")]
    async fn list_stale_wiki_pages(&self) -> Result<CallToolResult, McpError> {
        let pairs = self
            .services
            .page_ref_store
            .list_stale_wiki_pages()
            .await
            .map_err(internal)?;
        let titles: std::collections::HashMap<String, String> = self
            .services
            .wiki_page_store
            .list()
            .await
            .map_err(internal)?
            .into_iter()
            .map(|p| (p.slug, p.title))
            .collect();
        // Pairs arrive ordered by slug then path, so consecutive rows
        // with the same slug group together.
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        for (slug, path) in pairs {
            match grouped.last_mut() {
                Some((s, refs)) if *s == slug => refs.push(path),
                _ => grouped.push((slug, vec![path])),
            }
        }
        let out: Vec<serde_json::Value> = grouped
            .into_iter()
            .map(|(slug, stale_refs)| {
                serde_json::json!({
                    "slug": &slug,
                    "title": titles.get(&slug).cloned().unwrap_or_default(),
                    "stale_refs": stale_refs,
                })
            })
            .collect();
        json_result(&out)
    }

    #[tool(description = "For one wiki page file ref, return the unified diff \
                       between the snapshot the ref was pinned to and the \
                       file's CURRENT on-disk content — so you can read just \
                       what drifted instead of re-opening the whole file. \
                       Pair with `list_stale_wiki_pages` / \
                       `get_wiki_page_metadata.stale_refs` to find which \
                       (slug, path) drifted, then call this per ref. Returns \
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
            &self.services.blobs,
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
        let id = ThreadId::from(params.0.thread_id);
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
        let id = ThreadId::from(params.0.thread_id);
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
        Ok(CallToolResult::success(vec![Content::text("removed")]))
    }

    // ---------- subsystem docs ----------

    #[tool(description = "Read a `.context/<name>.md` subsystem doc; returns body + exists flag.")]
    async fn get_subsystem_doc(
        &self,
        params: Parameters<SubsystemDocParams>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .services
            .layout
            .project_dir
            .join(".context")
            .join(format!("{}.md", params.0.name));
        let exists = path.exists();
        let content = if exists {
            std::fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };
        let body = serde_json::json!({ "exists": exists, "content": content });
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }

    // ---------- task orchestration ----------

    #[tool(
        description = "Create a new task. Allocates id + sort_index, fires creation event. \
                       `thread_id` is required unless `backlog: true` is set (a thread-detached \
                       row trips filing-enforcement on the next edit, so backlog filing must be \
                       an explicit choice). Pass `status: \"in_progress\"` to start the work in \
                       the same call (filing-enforcement requires an in_progress row to exist \
                       before edits land). Pass `status: \"done\"` (or `blocked`) with \
                       `touched_files` to file a row for already-shipped work — the runtime \
                       synthesizes the in_progress→target effort so Local History attributes \
                       the writes."
    )]
    async fn create_task(
        &self,
        params: Parameters<CreateTaskMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match (p.thread_id.as_deref(), p.backlog) {
            (Some(_), true) => {
                return Err(McpError::invalid_params(
                    "create_task: pass `thread_id` OR `backlog: true`, not both",
                    None,
                ));
            }
            (None, false) => {
                return Err(McpError::invalid_params(
                    "create_task: `thread_id` is required (or set `backlog: true` to file \
                     onto the project-wide backlog)",
                    None,
                ));
            }
            _ => {}
        }
        if let Some(tid) = p.thread_id.as_deref() {
            expect_id_kind("create_task", "thread_id", tid, ID_THREAD)?;
        }
        let parent_task_id = match p.parent_id.as_deref() {
            Some(pid) => Some(parse_task_id("create_task", "parent_id", pid)?),
            None => None,
        };
        let thread = p.thread_id.clone().map(ThreadId::from);
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        let status = match p.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
            None => None,
        };
        let item = self
            .services
            .tasks
            .create(
                thread.clone(),
                CreateTaskInput {
                    title: p.title,
                    description: p.description,
                    description_executive: p.description_executive,
                    description_caveman: p.description_caveman,
                    parent_id: parent_task_id,
                    status,
                    priority,
                    author: Some(oxplow_domain::TaskAuthor::Agent),
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        // Synthesize the in_progress→target effort when the row was
        // filed directly into a closing state with touched files.
        // Mirrors main: a `done`/`blocked` create with `touchedFiles`
        // is the "file and close in one call" shortcut for retroactive
        // splits, and Local History needs the effort row to attribute
        // the writes to this item.
        let touched = p.touched_files.unwrap_or_default();
        if !touched.is_empty() && matches!(item.status, TaskStatus::Done | TaskStatus::Blocked) {
            let thread_for_effort = thread.or_else(|| item.thread_id.clone());
            if let Some(tid) = thread_for_effort {
                let worktree = worktree_for_thread(&self.services, &tid).await;
                if let Err(err) = self
                    .services
                    .tasks
                    .record_effort(
                        &self.services.effort_store,
                        item.id,
                        &tid,
                        &touched,
                        None,
                        &[],
                        worktree.as_deref(),
                    )
                    .await
                {
                    tracing::warn!(?err, "create_task: effort record failed");
                }
            }
        }
        self.emit_tasks_changed(item.thread_id.clone());
        json_result(&item)
    }

    #[tool(
        description = "Update fields on an existing task (partial-patch). Pass `touched_files` \
                       alongside a `status` transition to `done`/`blocked` to attribute the closing \
                       effort. `parent_id` reparents (empty string detaches)."
    )]
    async fn update_task(
        &self,
        params: Parameters<UpdateTaskMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = parse_task_id("update_task", "id", &p.id)?;
        if let Some(pid) = p.parent_id.as_deref() {
            // Empty string is the "detach" sentinel — only validate non-empty.
            if !pid.is_empty() {
                parse_task_id("update_task", "parent_id", pid)?;
            }
        }
        let status = match p.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
            None => None,
        };
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        // Parent: `Option<Option<…>>` semantics — outer Some means
        // "the field was passed", inner None means "clear it". Empty
        // string = clear; non-empty = set.
        let parent_id: Option<Option<TaskId>> = match p.parent_id {
            Some(s) if s.is_empty() => Some(None),
            Some(s) => Some(Some(parse_task_id("update_task", "parent_id", &s)?)),
            None => None,
        };
        let updated = self
            .services
            .tasks
            .update(
                id,
                UpdateTaskChanges {
                    title: p.title,
                    description: p.description,
                    description_executive: p.description_executive,
                    description_caveman: p.description_caveman,
                    parent_id,
                    status,
                    priority,
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let touched = p.touched_files.unwrap_or_default();
        if !touched.is_empty() && matches!(updated.status, TaskStatus::Done | TaskStatus::Blocked) {
            if let Some(tid) = updated.thread_id.clone() {
                let worktree = worktree_for_thread(&self.services, &tid).await;
                if let Err(err) = self
                    .services
                    .tasks
                    .record_effort(
                        &self.services.effort_store,
                        updated.id,
                        &tid,
                        &touched,
                        None,
                        &[],
                        worktree.as_deref(),
                    )
                    .await
                {
                    tracing::warn!(?err, "update_task: effort record failed");
                }
            }
        }
        self.emit_tasks_changed(updated.thread_id.clone());
        json_result(&updated)
    }

    #[tool(
        description = "Append a summary note to a task then mark it `done`. Pass \
                       `touched_files` (repo-relative paths edited for this effort) to attribute \
                       the writes via Local History — skip only if you edited >100 files. Pass \
                       `impacts` to declare what else this effort changed: wiki pages you \
                       created/updated/deleted, tasks you completed/reopened, commits you \
                       referenced, findings you resolved. Each is projected into the cross-page \
                       backlink graph so e.g. a new wiki page lists this task as its origin \
                       without relying on summary-body text parsing. \
                       \n\nReturns `{ task, file_review }`. When `file_review` is non-null \
                       the snapshot bracket diff disagreed with `touched_files`: \
                       `claimed_but_not_changed` lists files you said you edited but the \
                       worktree didn't change, and `changed_but_not_claimed` lists files \
                       that did change but you didn't list. Inspect both, then call \
                       `amend_effort(effort_id, add_files, remove_files)` to correct the \
                       attribution. If you genuinely meant your original list (e.g. you \
                       edited then reverted, or another actor changed those files), no \
                       amend is needed."
    )]
    async fn complete_task(
        &self,
        params: Parameters<CompleteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = parse_task_id("complete_task", "id", &p.id)?;
        let _ = p.author; // legacy field — kept on the wire, no longer attributed
        let item = self
            .services
            .tasks
            .update(
                id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let touched = p.touched_files.unwrap_or_default();
        let impacts: Vec<oxplow_domain::TaskImpact> = p
            .impacts
            .unwrap_or_default()
            .into_iter()
            .map(|i| oxplow_domain::TaskImpact {
                kind: i.kind,
                id: i.id,
                action: i.action,
            })
            .collect();
        let summary_has_body = !p.summary.trim().is_empty();
        let mut review: Option<oxplow_app::task_service::EffortFileReview> = None;
        if (summary_has_body || !touched.is_empty() || !impacts.is_empty())
            && item.thread_id.is_some()
        {
            let tid = item
                .thread_id
                .clone()
                .expect("thread_id present — guarded by is_some() above");
            let summary = if summary_has_body {
                Some(p.summary.clone())
            } else {
                None
            };
            let worktree = worktree_for_thread(&self.services, &tid).await;
            if let Err(err) = self
                .services
                .tasks
                .record_effort(
                    &self.services.effort_store,
                    item.id,
                    &tid,
                    &touched,
                    summary,
                    &impacts,
                    worktree.as_deref(),
                )
                .await
            {
                tracing::warn!(?err, "complete_task: effort record failed");
            } else {
                review = oxplow_app::task_service::compute_effort_file_review(
                    &self.services.effort_store,
                    &self.services.snapshot_store,
                    item.id,
                    &touched,
                )
                .await;
                // Stash the effort id so the Stop hook can fire a
                // one-shot directive prompting the agent to amend
                // (or silently agree). Recomputed at stop time so a
                // subsequent amend_effort that already reconciled
                // the discrepancy doesn't trigger a stale prompt.
                if let (Some(r), Some(tid)) = (review.as_ref(), item.thread_id.clone()) {
                    self.services.thread_runtime.record_pending_effort_review(
                        &tid,
                        oxplow_domain::EffortId::from(r.effort_id.clone()),
                    );
                }
                // Persist the executive/caveman summary variants onto
                // the effort that record_effort just wrote the summary
                // to (the developer text). The variant blob stays NULL
                // unless the agent authored a rewrite.
                if summary_has_body
                    && (p.summary_executive.is_some() || p.summary_caveman.is_some())
                {
                    use oxplow_db::TaskEffortStore as _;
                    if let Ok(Some(effort)) = self
                        .services
                        .effort_store
                        .most_recent_for_task(item.id)
                        .await
                    {
                        let variants = oxplow_domain::ProseVariants {
                            developer: p.summary.clone(),
                            executive: p.summary_executive.clone(),
                            caveman: p.summary_caveman.clone(),
                        };
                        if let Err(err) = self
                            .services
                            .effort_store
                            .set_summary_variants(&effort.id, &variants)
                            .await
                        {
                            tracing::warn!(?err, "complete_task: summary variant persist failed");
                        }
                    }
                }
            }
        }
        self.emit_tasks_changed(item.thread_id.clone());
        let payload = CompleteTaskResult {
            task: item,
            file_review: review,
        };
        json_result(&payload)
    }

    #[tool(
        description = "Adjust an effort's `task_effort_file` rows after the fact. Use to \
                       reconcile the file-attribution list when the auto-diff disagreed \
                       with your declared `touched_files` on `complete_task`. \
                       `add_files` claims paths the diff thought weren't yours; \
                       `remove_files` disclaims paths the diff thought were yours but \
                       actually came from another actor (formatter, parallel effort, the \
                       user). Either list may be empty/omitted; passing both empty is a no-op."
    )]
    async fn amend_effort(
        &self,
        params: Parameters<AmendEffortParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_db::TaskEffortStore as _;
        let p = params.0;
        let effort_id = oxplow_domain::EffortId::from(p.effort_id);
        let add = p.add_files.unwrap_or_default();
        let remove = p.remove_files.unwrap_or_default();
        for path in &remove {
            if path.is_empty() {
                continue;
            }
            self.services
                .effort_store
                .remove_file(&effort_id, path)
                .await
                .map_err(|e| internal(e.to_string()))?;
            // Record the disclaim as an explicit acknowledgement so
            // the Stop hook's recompute doesn't re-flag the same
            // `changed_but_not_claimed` discrepancy. Survives across
            // turns; cleared if the agent later re-claims the path
            // via `add_files`.
            self.services
                .effort_store
                .acknowledge_unclaimed_path(&effort_id, path)
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
        // Compute the snapshot-version pin once for this effort —
        // every added path inherits the same triple. Falls back to a
        // 0 snapshot id when the effort has no snapshot pin (rare),
        // matching the policy used by `record_effort`.
        let effort = self
            .services
            .effort_store
            .get_effort(&effort_id)
            .await
            .map_err(|e| internal(e.to_string()))?;
        let version = if let Some(effort) = effort {
            self.services
                .tasks
                .resolve_effort_file_version(&effort)
                .await
        } else {
            oxplow_app::file_ref_version::ResolvedFileVersion {
                local_snapshot_id: 0,
                closest_git_version: None,
                git_version_exact: false,
            }
        };
        for path in &add {
            if path.is_empty() {
                continue;
            }
            // change_kind defaults to Updated — the agent's amend
            // doesn't carry stat info, and the per-file change kind
            // is informational only (UI shows it; backlinks don't
            // discriminate).
            self.services
                .effort_store
                .record_file(
                    &effort_id,
                    path,
                    oxplow_db::EffortFileChange::Updated,
                    version.as_ref(),
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            // If this path was previously acknowledged-as-not-mine
            // (i.e. disclaimed), clear that acknowledgement now that
            // the agent has changed their mind and is claiming it.
            self.services
                .effort_store
                .forget_acknowledged_path(&effort_id, path)
                .await
                .map_err(|e| internal(e.to_string()))?;
        }
        json_result(&serde_json::json!({
            "effort_id": effort_id.as_str(),
            "added": add,
            "removed": remove,
        }))
    }

    #[tool(description = "Create a typed link between two tasks.")]
    async fn link_tasks(
        &self,
        params: Parameters<LinktasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("link_tasks", "thread_id", &p.thread_id, ID_THREAD)?;
        let from_id = parse_task_id("link_tasks", "from_id", &p.from_id)?;
        let to_id = parse_task_id("link_tasks", "to_id", &p.to_id)?;
        let link_type = parse_link_type(&p.link_type)?;
        let thread = ThreadId::from(p.thread_id);
        let link = self
            .services
            .task_link_store
            .create(&thread, from_id, to_id, link_type)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(Some(thread));
        json_result(&link)
    }

    #[tool(description = "Transition a batch of tasks to the same status.")]
    async fn transition_tasks(
        &self,
        params: Parameters<TransitiontasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mut parsed_ids: Vec<TaskId> = Vec::with_capacity(p.ids.len());
        for raw in &p.ids {
            parsed_ids.push(parse_task_id("transition_tasks", "ids[]", raw)?);
        }
        let target = parse_status(&p.status)?;
        let mut updated = Vec::with_capacity(parsed_ids.len());
        for id in parsed_ids {
            let row = self
                .services
                .tasks
                .update(
                    id,
                    UpdateTaskChanges {
                        status: Some(target),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            updated.push(row);
        }
        let mut threads: std::collections::HashSet<Option<oxplow_domain::ThreadId>> =
            std::collections::HashSet::new();
        for row in &updated {
            threads.insert(row.thread_id.clone());
        }
        for tid in threads {
            self.emit_tasks_changed(tid);
        }
        json_result(&updated)
    }

    #[tool(
        description = "Signal that the agent is awaiting user input. Persists a hook event so Stop suppression kicks in."
    )]
    async fn await_user(
        &self,
        params: Parameters<AwaitUserParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("await_user", "thread_id", &p.thread_id, ID_THREAD)?;
        let payload = serde_json::json!({
            "await_user": true,
            "question": p.question,
        })
        .to_string();
        let event = oxplow_domain::HookEvent {
            id: oxplow_domain::HookEventId::new(),
            thread_id: Some(ThreadId::from(p.thread_id.clone())),
            stream_id: None,
            kind: oxplow_domain::HookKind::Stop,
            session_id: None,
            payload_json: payload,
            received_at: oxplow_domain::Timestamp::now(),
        };
        self.services
            .hook_event_store
            .append(&event)
            .await
            .map_err(internal)?;
        // Flip the agent_status to AwaitingUser directly so the
        // renderer reflects the state without needing a Stop hook.
        self.services
            .agent_status_store
            .upsert(
                &ThreadId::from(p.thread_id),
                "working",
                oxplow_domain::AgentStatusState::AwaitingUser,
                Some("await_user".into()),
            )
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("awaiting")]))
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
        let id = ThreadId::from(params.0.thread_id);
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
        let events = self
            .services
            .task_event_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        let bundle = serde_json::json!({
            "thread": thread,
            "items": items,
            "events": events,
        });
        Ok(CallToolResult::success(vec![Content::text(
            bundle.to_string(),
        )]))
    }

    #[tool(description = "Atomic: create an epic plus a list of children attached to it.")]
    async fn file_epic_with_children(
        &self,
        params: Parameters<FileEpicWithChildrenParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        if let Some(t) = p.thread_id.as_deref() {
            expect_id_kind("file_epic_with_children", "thread_id", t, ID_THREAD)?;
        }
        let thread = p.thread_id.map(ThreadId::from);
        let epic = self
            .services
            .tasks
            .create(
                thread.clone(),
                CreateTaskInput {
                    title: p.epic_title,
                    description: p.epic_description,
                    description_executive: p.epic_description_executive,
                    description_caveman: p.epic_description_caveman,
                    author: Some(oxplow_domain::TaskAuthor::Agent),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        let mut children_out = Vec::with_capacity(p.children.len());
        for child in p.children {
            let row = self
                .services
                .tasks
                .create(
                    thread.clone(),
                    CreateTaskInput {
                        title: child.title,
                        description: child.description,
                        description_executive: child.description_executive,
                        description_caveman: child.description_caveman,
                        parent_id: Some(epic.id),
                        author: Some(oxplow_domain::TaskAuthor::Agent),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            children_out.push(row);
        }
        self.emit_tasks_changed(thread.clone());
        let bundle = serde_json::json!({ "epic": epic, "children": children_out });
        Ok(CallToolResult::success(vec![Content::text(
            bundle.to_string(),
        )]))
    }

    #[tool(
        description = "Compose a ready-to-paste dispatch brief for a task and transition it \
                       to in_progress in one atomic call. When `item_id` is given, dispatches that \
                       specific item; otherwise picks the first ready non-epic item on the thread \
                       (mirrors main's /work-next composition shortcut). Returns \
                       `{ ok, prompt, itemId }` — pass `prompt` to the general-purpose Agent tool. \
                       The brief carries the item fields, AC, recent notes, and the subagent \
                       protocol preamble so the orchestrator brief stays slim."
    )]
    async fn dispatch_task(
        &self,
        params: Parameters<DispatchTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("dispatch_task", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let parsed_item_id = match params.0.item_id.as_deref() {
            Some(raw) => Some(parse_task_id("dispatch_task", "item_id", raw)?),
            None => None,
        };
        let thread_id = ThreadId::from(params.0.thread_id.clone());
        let target = match parsed_item_id {
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
            None => {
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

        let updated = self
            .services
            .tasks
            .update(
                target.id,
                oxplow_app::UpdateTaskChanges {
                    status: Some(oxplow_domain::TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let prompt =
            compose_dispatch_brief(&updated, params.0.extra_context.as_deref().unwrap_or(""));
        self.emit_tasks_changed(updated.thread_id.clone());
        json_result(&serde_json::json!({
            "ok": true,
            "prompt": prompt,
            "itemId": updated.id,
        }))
    }

    #[tool(
        description = "Branch a new thread off an existing one (shared stream, fresh thread row)."
    )]
    async fn fork_thread(
        &self,
        params: Parameters<ForkThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "fork_thread",
            "source_thread_id",
            &params.0.source_thread_id,
            ID_THREAD,
        )?;
        let source = ThreadId::from(params.0.source_thread_id);
        let parent = self
            .services
            .thread_store
            .get(&source)
            .await
            .map_err(internal)?
            .ok_or_else(|| McpError::invalid_params("source thread not found", None))?;
        let child = self
            .services
            .threads
            .create(&parent.stream_id, params.0.title, parent.pane_target)
            .await
            .map_err(|e| internal(e.to_string()))?;
        json_result(&child)
    }

    #[tool(
        description = "Unified backlinks: every page (wiki, task, commit, finding, \
                       …) that points AT the given target page. The target is identified \
                       by `kind` (e.g. \"file\", \"wiki\", \"task\", \"git-commit\", \
                       \"finding\", \"directory\") and `id` (path / slug / wi-… / sha / id). \
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
        description = "Wiki pages that reference the given note slug in their related_notes \
                       (from [[other-note-slug]] wikilinks). Use for note-to-note backlinks."
    )]
    async fn find_wiki_pages_for_wiki_page(
        &self,
        params: Parameters<FindNotesForNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut hits = oxplow_app::wiki_pages::backlinks_for_note(
            &self.services.wiki_page_store,
            &params.0.slug,
        )
        .await
        .map_err(internal)?;
        if (params.0.limit as usize) > 0 && hits.len() > params.0.limit as usize {
            hits.truncate(params.0.limit as usize);
        }
        json_result(&hits)
    }

    // ---------- LSP ----------

    #[tool(description = "LSP textDocument/definition for a position in a file.")]
    async fn lsp_definition(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/hover for a position in a file.")]
    async fn lsp_hover(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/references for a position in a file.")]
    async fn lsp_references(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                    "context": { "includeDeclaration": true },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/diagnostic — pulls the latest diagnostics for a file.")]
    async fn lsp_diagnostics(
        &self,
        params: Parameters<LspDiagnosticsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/diagnostic",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "Re-read a wiki page's body file and refresh the FTS index.")]
    async fn resync_wiki_page(
        &self,
        params: Parameters<ResyncNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let slug = params.0.slug;
        let mut note = self
            .services
            .wiki_page_store
            .get(&slug)
            .await
            .map_err(internal)?
            .ok_or_else(|| McpError::invalid_params(format!("note not found: {slug}"), None))?;
        let body_path = self
            .services
            .layout
            .project_dir
            .join(".oxplow")
            .join("wiki")
            .join(format!("{slug}.md"));
        let body = std::fs::read_to_string(&body_path).unwrap_or_default();
        // Refresh excerpt + size; upsert re-syncs the FTS mirror.
        note.body_excerpt = body.chars().take(500).collect();
        note.body_size_bytes = body.len() as i64;
        note.updated_at = oxplow_domain::Timestamp::now();
        self.services
            .wiki_page_store
            .upsert(&note)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

    #[tool(description = "Record a wiki page edit's freshness bookkeeping. \
                       Call this AFTER editing a `.oxplow/wiki/<slug>.md` \
                       file. Both `verified_refs` and `removed_refs` are \
                       REQUIRED: pass `[]` if nothing applies, but be \
                       explicit so the freshness signal stays honest. \
                       `verified_refs` lists repo-relative file paths you \
                       re-read against the new body during this edit \
                       (those refs get their snapshot pin advanced to \
                       current). A verified path may be a file the body \
                       cites directly OR a file under a directory the \
                       body cites via `[[dir:…]]` — verifying a fact \
                       against `crates/x/src/lib.rs` when the page only \
                       cites `[[dir:crates/x]]` now records a precise \
                       pin for that file. `removed_refs` lists paths you \
                       intentionally removed from the page (validated to \
                       no longer appear in the body). Refs left in place \
                       without re-checking should appear in NEITHER list \
                       — they keep their existing pin so 'this content \
                       relies on stale sources' surfaces accurately.")]
    async fn record_wiki_page_update(
        &self,
        params: Parameters<RecordWikiPageUpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        use oxplow_app::file_ref_version;
        use oxplow_db::page_ref_projections::{KIND_FILE, KIND_WIKI, RT_WIKI_FILE};
        let p = params.0;
        let slug = p.slug;
        // Force a synchronous re-sync of the wiki page so the
        // page_ref state matches the on-disk body before we
        // validate / re-stamp. The fs-watcher will run again later
        // but that's a no-op merge.
        let resolved_version = {
            // Wiki pages are project-wide; freshness is tagged against
            // the primary stream's snapshot service. Pseudo-stream
            // migration tracked in epic #28's follow-up.
            let Some(svc) = self.services.snapshot_captures.primary() else {
                return Err(internal("primary snapshot service not registered"));
            };
            let stream_id = oxplow_domain::StreamId::from(svc.stream_id().to_string());
            match svc.store().latest_snapshot_id_for_stream(stream_id).await {
                Ok(Some(snapshot_id)) => {
                    file_ref_version::resolve(svc.store(), svc.project_dir(), snapshot_id)
                        .await
                        .ok()
                }
                _ => None,
            }
        };
        oxplow_app::wiki_pages::sync_from_disk_with_refs_versioned(
            &self.services.layout.project_dir,
            &self.services.wiki_page_store,
            Some(&self.services.page_ref_store),
            &slug,
            resolved_version.clone(),
        )
        .await
        .map_err(internal)?;
        // Read the body now reflected in the DB to validate against
        // the agent's declarations.
        let body_path = self
            .services
            .layout
            .project_dir
            .join(".oxplow")
            .join("wiki")
            .join(format!("{slug}.md"));
        let body = std::fs::read_to_string(&body_path)
            .map_err(|e| internal(format!("read {slug}.md: {e}")))?;
        let parsed = oxplow_app::wiki_pages::parse_refs(&body);
        let body_files: std::collections::HashSet<&str> =
            parsed.file_refs.iter().map(|s| s.as_str()).collect();
        // removed_refs MUST NOT appear in body.
        let still_present: Vec<&String> = p
            .removed_refs
            .iter()
            .filter(|path| body_files.contains(path.as_str()))
            .collect();
        if !still_present.is_empty() {
            return Err(McpError::invalid_params(
                format!(
                    "removed_refs entries still referenced by the body: {}",
                    still_present
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None,
            ));
        }
        // A verified_ref is acceptable if the body cites it directly
        // as a file, OR it's a file living under a directory the body
        // cites (`[[dir:…]]`) — the "I verified a fact against a file
        // I reference only by its directory" case. Anything else is
        // genuinely unreferenced and rejected.
        let missing: Vec<&String> = p
            .verified_refs
            .iter()
            .filter(|path| {
                !body_files.contains(path.as_str())
                    && !oxplow_app::wiki_pages::path_under_any_dir(path, &parsed.dir_refs)
            })
            .collect();
        if !missing.is_empty() {
            return Err(McpError::invalid_params(
                format!(
                    "verified_refs entries are neither referenced by the body nor under a \
                     cited directory: {}. Reference the file in the body, or name a file \
                     under a [[dir:…]] the page cites.",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None,
            ));
        }
        // Pin verified refs to the current snapshot. A body file ref
        // re-stamps its existing edge; a file under a cited dir is
        // *materialized* as a new `(wiki→file)` edge (the wiki sync
        // preserves it as long as the dir ref stays). Without a
        // resolved snapshot (no snapshot service, e.g. tests) the body
        // ref can't be re-stamped, but a materialized edge is still
        // created — unpinned, which the staleness signal treats as
        // stale until a real snapshot lands.
        let mut restamped = Vec::new();
        for path in &p.verified_refs {
            if body_files.contains(path.as_str()) {
                if let Some(v) = resolved_version.as_ref() {
                    self.services
                        .page_ref_store
                        .restamp_edge_version(
                            KIND_WIKI,
                            &slug,
                            KIND_FILE,
                            path,
                            RT_WIKI_FILE,
                            v.local_snapshot_id,
                            v.closest_git_version.clone(),
                            v.git_version_exact,
                        )
                        .await
                        .map_err(|e| internal(e.to_string()))?;
                }
            } else {
                let mut edge =
                    oxplow_db::PageRefEdge::new(KIND_WIKI, &slug, KIND_FILE, path, RT_WIKI_FILE);
                if let Some(v) = resolved_version.as_ref() {
                    edge = edge.with_version(
                        v.local_snapshot_id,
                        v.closest_git_version.clone(),
                        v.git_version_exact,
                    );
                }
                self.services
                    .page_ref_store
                    .upsert_edge(edge)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
            }
            restamped.push(path.clone());
        }
        json_result(&serde_json::json!({
            "slug": slug,
            "verified": restamped,
            "removed": p.removed_refs,
        }))
    }
}

fn parse_status(s: &str) -> Result<TaskStatus, McpError> {
    Ok(match s {
        "ready" => TaskStatus::Ready,
        "in_progress" => TaskStatus::InProgress,
        "blocked" => TaskStatus::Blocked,
        "done" => TaskStatus::Done,
        "canceled" => TaskStatus::Canceled,
        "archived" => TaskStatus::Archived,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown task status: {other}"),
                None,
            ))
        }
    })
}

fn parse_priority(s: &str) -> Result<oxplow_domain::TaskPriority, McpError> {
    use oxplow_domain::TaskPriority as P;
    Ok(match s {
        "low" => P::Low,
        "medium" => P::Medium,
        "high" => P::High,
        "urgent" => P::Urgent,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown priority: {other}"),
                None,
            ))
        }
    })
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

async fn resolve_lsp_proxy(
    services: &Services,
    stream_id: &str,
    language: &str,
) -> Result<std::sync::Arc<oxplow_app::LspProxy>, McpError> {
    expect_id_kind("lsp", "stream_id", stream_id, ID_STREAM)?;
    let stream = services
        .streams
        .list_streams()
        .await
        .map_err(|e| internal(e.to_string()))?
        .into_iter()
        .find(|s| s.id.as_str() == stream_id)
        .ok_or_else(|| McpError::invalid_params(format!("stream not found: {stream_id}"), None))?;
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    services
        .lsp_sessions
        .ensure(stream_id, language, cwd)
        .await
        .map_err(|e| internal(e.to_string()))
}

fn parse_link_type(s: &str) -> Result<TaskLinkType, McpError> {
    Ok(match s {
        "blocks" => TaskLinkType::Blocks,
        "relates_to" => TaskLinkType::RelatesTo,
        "discovered_from" => TaskLinkType::DiscoveredFrom,
        "duplicates" => TaskLinkType::Duplicates,
        "supersedes" => TaskLinkType::Supersedes,
        "replies_to" => TaskLinkType::RepliesTo,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown link type: {other}"),
                None,
            ))
        }
    })
}

#[tool_handler]
impl ServerHandler for OxplowMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Oxplow MCP server. Exposes task, note, wiki, and stream surfaces. \
                 Authoritative tool list lives at .context/agent-model.md."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
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

/// Map a coverage-ingest outcome to a JSON status the agent can act on.
fn ingest_outcome_json(outcome: &oxplow_app::collection::CoverageIngest) -> serde_json::Value {
    use oxplow_app::collection::CoverageIngest as C;
    match outcome {
        C::NoOpenEffort => serde_json::json!({ "status": "no_open_effort" }),
        C::NotConfigured => serde_json::json!({
            "status": "not_configured",
            "hint": "set collection.coverageReportPath + coverageFormat in oxplow.yaml (run /oxplow:configure)",
        }),
        C::ReportMissing(path) => serde_json::json!({ "status": "report_missing", "path": path }),
        C::StaleReport(path) => serde_json::json!({ "status": "stale_report", "path": path }),
        C::ParseError(err) => serde_json::json!({ "status": "parse_error", "error": err }),
        C::NoBaseline => serde_json::json!({ "status": "no_baseline" }),
        C::NoChangedCoverage => serde_json::json!({ "status": "no_changed_coverage" }),
        C::Stored {
            observation_id,
            summary_pct,
            changed_lines,
            covered_lines,
        } => serde_json::json!({
            "status": "stored",
            "observationId": observation_id,
            "summaryPct": summary_pct,
            "changedLines": changed_lines,
            "coveredLines": covered_lines,
        }),
    }
}

/// Validate an optional `stream_id`: enforce the `s-` prefix when present,
/// and accept `None` (resolves to the current/primary worktree downstream).
fn check_optional_stream(tool: &str, stream_id: Option<&str>) -> Result<(), McpError> {
    match stream_id {
        Some(id) => expect_id_kind(tool, "stream_id", id, ID_STREAM),
        None => Ok(()),
    }
}

/// Parse a `note`/`followup` string into a `CommentIntent`.
fn parse_comment_intent(tool: &str, value: &str) -> Result<oxplow_domain::CommentIntent, McpError> {
    match value.to_ascii_lowercase().as_str() {
        "note" => Ok(oxplow_domain::CommentIntent::Note),
        "followup" => Ok(oxplow_domain::CommentIntent::Followup),
        other => Err(McpError::invalid_params(
            format!("{tool}: `intent` expects `note` or `followup`, got `{other}`"),
            None,
        )),
    }
}

/// A wiki file ref is stale when its path has been snapshotted more
/// recently than the ref's captured pin, or it was never pinned but the
/// file has a snapshot. Mirrors the per-ref rule in the UI's
/// `list_wiki_freshness` reader and the SQL in
/// `SqlitePageRefStore::list_stale_wiki_pages`.
fn wiki_ref_stale(local: Option<i64>, latest: Option<i64>) -> bool {
    matches!((latest, local), (Some(l), Some(loc)) if l > loc)
        || matches!((latest, local), (Some(_), None))
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
    match oxplow_domain::TaskId::try_from_str(value) {
        Some(id) => Ok(id),
        None => Err(McpError::invalid_params(
            format!("{tool}: `{param}` expects a task id (integer), got `{value}`"),
            None,
        )),
    }
}

/// String-id prefix validator. Tasks now have integer ids and go
/// through [`parse_task_id`]; everything else still carries a
/// `<prefix>-<rest>` shape, and this helper confirms a caller-supplied
/// value matches the prefix the tool wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdPrefix {
    pub prefix: &'static str,
    pub label: &'static str,
}

pub(crate) const ID_STREAM: IdPrefix = IdPrefix {
    prefix: "s-",
    label: "stream id (s-…)",
};
pub(crate) const ID_THREAD: IdPrefix = IdPrefix {
    prefix: "b-",
    label: "thread id (b-…)",
};
pub(crate) const ID_NOTE: IdPrefix = IdPrefix {
    prefix: "n-",
    label: "note id (n-…)",
};
pub(crate) const ID_FOLLOWUP: IdPrefix = IdPrefix {
    prefix: "fu-",
    label: "follow-up id (fu-…)",
};

fn expect_id_kind(
    tool: &str,
    param: &str,
    value: &str,
    expected: IdPrefix,
) -> Result<(), McpError> {
    if value.starts_with(expected.prefix) && value.len() > expected.prefix.len() {
        return Ok(());
    }
    // Tell the caller what the value *looks* like so they can correct
    // an "I passed a thread id where a stream id was expected" mix-up
    // without a second round-trip.
    let actual_label = match value.split_once('-') {
        Some(("s", _)) => "stream id (s-…)",
        Some(("b", _)) => "thread id (b-…)",
        Some(("n", _)) => "note id (n-…)",
        Some(("fu", _)) => "follow-up id (fu-…)",
        Some(("at", _)) => "agent-turn id (at-…)",
        Some(("he", _)) => "hook-event id (he-…)",
        Some(("ef", _)) => "effort id (ef-…)",
        Some(("pv", _)) => "page-visit id (pv-…)",
        Some(("ue", _)) => "usage-event id (ue-…)",
        Some(("bg", _)) => "background-task id (bg-…)",
        Some(_) => "id with an unrecognised prefix",
        None => "value with no `<prefix>-…` shape",
    };
    let msg = format!(
        "{tool}: `{param}` expects a {expected_label}, but got `{value}` which looks like a \
         {actual_label}",
        tool = tool,
        param = param,
        expected_label = expected.label,
        value = value,
    );
    Err(McpError::invalid_params(msg, None))
}

/// Compose the prompt the orchestrator passes to
/// `Agent(subagent_type='Explore', prompt=…)`. Pure so it's
/// testable without an MCP server. Mirrors `composeDelegateQueryPrompt`
/// from `src/mcp/mcp-tools.ts`.
fn compose_delegate_query_prompt(
    thread_id: &str,
    question: &str,
    focus: &str,
    note_id: &str,
) -> String {
    let mut parts: Vec<String> = vec![
        "You are an Explore subagent answering one focused exploration question for the orchestrator.".into(),
        String::new(),
        format!("threadId: {thread_id}"),
        format!("noteId: {note_id}"),
        String::new(),
        "## Question".into(),
        question.to_string(),
    ];
    if !focus.is_empty() {
        parts.push(String::new());
        parts.push("## Focus".into());
        parts.push(focus.to_string());
    }
    parts.push(String::new());
    parts.push("## How to report".into());
    parts.push(
        "When done, call `mcp__oxplow__record_query_finding({ noteId, body })` ONCE with your complete finding. \
         The body should be concise, structured prose — file paths, key function names, and the direct answer to the question. \
         Do not make code changes. Do not create tasks. Read/Grep/Glob only."
            .into(),
    );
    parts.join("\n")
}

/// Compose the brief the orchestrator passes to the general-purpose
/// Agent tool to dispatch a task to a subagent. Pure so it's
/// testable.
///
/// Sections: identity, description, AC, optional extra context, and
/// the closing reminder pointing at the subagent-protocol skill.
/// Per-item notes used to render here too but were retired —
/// task_effort.summary already records what shipped on prior
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
    out.push(
        "Follow the `oxplow-subagent-work-protocol` skill: mark in_progress on entry; \
         done on exit. Return ONE line: `oxplow-result: {\"ok\":true,\"itemId\":\"<id>\",…}`. \
         Pass `touched_files` to `complete_task` so Local History attributes the writes."
            .into(),
    );
    out.join("\n")
}

/// `complete_task` wire shape — the task plus an optional review
/// payload when the agent's `touched_files` claim disagreed with
/// the snapshot bracket diff.
#[derive(Debug, serde::Serialize)]
pub struct CompleteTaskResult {
    pub task: oxplow_domain::Task,
    pub file_review: Option<oxplow_app::task_service::EffortFileReview>,
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
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
    use rmcp::handler::server::wrapper::Parameters;

    fn boot() -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        let project = tempfile::tempdir().unwrap();
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
            description_variants: oxplow_domain::ProseVariants::default(),
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
    async fn list_comments_enriches_primary_and_context() {
        use oxplow_domain::comment::{CommentIntent, CommentTarget};
        let (_proj, services, server) = boot();
        let stream = services.streams.list_streams().await.unwrap()[0].id.clone();

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
            .comment_store
            .create(
                &stream,
                None,
                &CommentTarget {
                    kind: "task".into(),
                    id: primary_task.to_string(),
                },
                "the highlighted text",
                "[]",
                &[CommentTarget {
                    kind: "task".into(),
                    id: parent_task.to_string(),
                }],
                &[CommentTarget {
                    kind: "file".into(),
                    id: "src/app.rs".into(),
                }],
                CommentIntent::Followup,
                "user",
                "what about this?",
            )
            .await
            .unwrap();

        let r = server
            .list_comments(Parameters(ListCommentsParams {
                scope: "stream".into(),
                id: stream.to_string(),
                status: None,
            }))
            .await
            .unwrap();
        let body = text_payload(r);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let row = &parsed.as_array().unwrap()[0];

        // Primary target resolved to the task title.
        assert_eq!(row["primary"]["kind"], "task");
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
    async fn get_info_advertises_tool_capability() {
        let (_proj, _svc, server) = boot();
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
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

        let r = server.list_backlog().await.unwrap();
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
    async fn delete_task_soft_deletes() {
        let (_proj, services, server) = boot();
        let item = make_task(None, "to delete");
        let id = services.task_store.insert(&item).await.unwrap();

        server
            .delete_task(Parameters(TaskIdParams { id: id.to_string() }))
            .await
            .unwrap();

        // Soft-deleted: list_backlog should no longer include it.
        let r = server.list_backlog().await.unwrap();
        let body = text_payload(r);
        assert!(
            !body.contains(&format!("\"id\":{}", id.value())),
            "soft-deleted item should not appear in backlog: {body}",
        );
    }

    #[tokio::test]
    async fn amend_effort_adds_and_removes_files() {
        use oxplow_db::{EffortFileChange, TaskEffortStore as _};
        use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
        let (_proj, services, server) = boot();
        // Reuse the writer thread that boot's primary stream created.
        let stream = services.stream_store.list().await.unwrap().pop().unwrap();
        let thread = services
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("primary stream must have a writer thread");
        let mut item = make_task(Some(thread.id.clone()), "amend test");
        let task_id = services.task_store.insert(&item).await.unwrap();
        item.id = task_id;
        // Open an effort for this task with a pre-recorded file.
        let effort = services
            .effort_store
            .start(task_id, &thread.id, None)
            .await
            .unwrap();
        let v = oxplow_db::FileRefVersion {
            local_snapshot_id: 0,
            closest_git_version: None,
            git_version_exact: false,
        };
        services
            .effort_store
            .record_file(&effort.id, "src/keep.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();
        services
            .effort_store
            .record_file(&effort.id, "src/disclaim.rs", EffortFileChange::Updated, v)
            .await
            .unwrap();

        // Disclaim disclaim.rs, claim a new file.
        server
            .amend_effort(Parameters(AmendEffortParams {
                effort_id: effort.id.as_str().to_string(),
                add_files: Some(vec!["src/added.rs".into()]),
                remove_files: Some(vec!["src/disclaim.rs".into()]),
            }))
            .await
            .unwrap();

        let files = services.effort_store.list_files(&effort.id).await.unwrap();
        let paths: std::collections::BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["src/added.rs", "src/keep.rs"]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
        // Disclaimed paths land in the acknowledgement table so
        // the Stop hook's recompute treats them as resolved
        // discrepancies and stops re-firing the directive.
        let acks = services
            .effort_store
            .list_acknowledged_paths(&effort.id)
            .await
            .unwrap();
        assert_eq!(acks, vec!["src/disclaim.rs".to_string()]);
        // Re-claiming an acknowledged path should clear its ack.
        server
            .amend_effort(Parameters(AmendEffortParams {
                effort_id: effort.id.as_str().to_string(),
                add_files: Some(vec!["src/disclaim.rs".into()]),
                remove_files: None,
            }))
            .await
            .unwrap();
        let acks_after = services
            .effort_store
            .list_acknowledged_paths(&effort.id)
            .await
            .unwrap();
        assert!(
            acks_after.is_empty(),
            "re-claiming the path should clear its acknowledgement, got {acks_after:?}",
        );
    }

    /// Helper to write a wiki body and call record_wiki_page_update.
    async fn seed_wiki(project: &std::path::Path, slug: &str, body: &str) {
        let wiki_dir = project.join(".oxplow").join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(wiki_dir.join(format!("{slug}.md")), body).unwrap();
    }

    #[tokio::test]
    async fn record_wiki_page_update_validates_removed_refs_absent_from_body() {
        let (proj, _svc, server) = boot();
        seed_wiki(
            proj.path(),
            "intro",
            "see [[crates/foo.rs]] and [[crates/bar.rs]]",
        )
        .await;
        let err = server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec![],
                removed_refs: vec!["crates/foo.rs".into()],
            }))
            .await
            .expect_err("foo.rs still in body, should error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("crates/foo.rs"),
            "error should mention the offending path: {msg}",
        );
    }

    #[tokio::test]
    async fn record_wiki_page_update_validates_verified_refs_present_in_body() {
        let (proj, _svc, server) = boot();
        seed_wiki(proj.path(), "intro", "no refs here").await;
        let err = server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec!["crates/foo.rs".into()],
                removed_refs: vec![],
            }))
            .await
            .expect_err("foo.rs not in body, should error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("crates/foo.rs"),
            "error should mention the missing path: {msg}",
        );
    }

    #[tokio::test]
    async fn record_wiki_page_update_accepts_empty_lists() {
        // Empty verified + removed is allowed — the agent declared
        // "I didn't re-check or remove anything in this edit." Body
        // sync still happens, but no re-stamp.
        let (proj, _svc, server) = boot();
        seed_wiki(proj.path(), "intro", "see [[crates/foo.rs]]").await;
        server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec![],
                removed_refs: vec![],
            }))
            .await
            .expect("empty lists are allowed");
    }

    #[tokio::test]
    async fn wiki_ref_drift_reports_status_per_ref() {
        let (proj, _svc, server) = boot();
        seed_wiki(proj.path(), "intro", "see [[crates/foo.rs]]").await;
        server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec![],
                removed_refs: vec![],
            }))
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

    #[tokio::test]
    async fn record_wiki_page_update_accepts_file_under_cited_dir_and_materializes_edge() {
        let (proj, services, server) = boot();
        // Body cites the directory, not the file.
        seed_wiki(proj.path(), "intro", "see [[dir:crates/cp]] for details").await;
        server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec!["crates/cp/src/lib.rs".into()],
                removed_refs: vec![],
            }))
            .await
            .expect("a file under a cited dir is an acceptable verified_ref");
        // The verification edge was materialized as a wiki→file edge.
        let backlinks = services
            .page_ref_store
            .list_backlinks("file", "crates/cp/src/lib.rs", None)
            .await
            .unwrap();
        assert!(
            backlinks.iter().any(|e| e.source_id == "intro"),
            "expected a materialized wiki:intro → file edge, got {backlinks:?}"
        );
    }

    #[tokio::test]
    async fn record_wiki_page_update_rejects_file_not_under_any_cited_dir() {
        let (proj, _svc, server) = boot();
        seed_wiki(proj.path(), "intro", "see [[dir:crates/cp]] only").await;
        let err = server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec!["crates/other/src/main.rs".into()],
                removed_refs: vec![],
            }))
            .await
            .expect_err("a file outside every cited dir must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("crates/other/src/main.rs"),
            "error should name the offending path: {msg}"
        );
    }

    #[tokio::test]
    async fn get_wiki_page_metadata_includes_stale_refs_field() {
        // The enriched metadata carries `stale_refs` (empty here — no
        // snapshots seeded), which is what distinguishes it from the
        // bulk `list_wiki_pages` payload.
        let (proj, _svc, server) = boot();
        seed_wiki(proj.path(), "intro", "see [[crates/foo.rs]]").await;
        server
            .record_wiki_page_update(Parameters(RecordWikiPageUpdateParams {
                slug: "intro".into(),
                verified_refs: vec![],
                removed_refs: vec![],
            }))
            .await
            .unwrap();
        let r = server
            .get_wiki_page_metadata(Parameters(SlugParams {
                slug: "intro".into(),
            }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(
            body.contains("\"stale_refs\""),
            "metadata should carry stale_refs: {body}"
        );
    }

    #[test]
    fn wiki_ref_stale_rule() {
        assert!(
            wiki_ref_stale(Some(100), Some(200)),
            "newer snapshot is stale"
        );
        assert!(!wiki_ref_stale(Some(200), Some(200)), "equal is fresh");
        assert!(
            !wiki_ref_stale(Some(200), Some(50)),
            "older file snapshot is fresh"
        );
        assert!(
            wiki_ref_stale(None, Some(50)),
            "unpinned but snapshotted is stale"
        );
        assert!(
            !wiki_ref_stale(Some(100), None),
            "no snapshot for path is fresh"
        );
        assert!(!wiki_ref_stale(None, None), "neither is fresh");
    }

    #[tokio::test]
    async fn create_task_rejects_stream_id_passed_as_thread_id() {
        let (_proj, _svc, server) = boot();
        let err = server
            .create_task(Parameters(CreateTaskMcpParams {
                thread_id: Some("s-deadbeef".into()),
                backlog: false,
                title: "x".into(),
                description: None,
                description_executive: None,
                description_caveman: None,
                kind: None,
                priority: None,
                status: None,
                parent_id: None,
                touched_files: None,
            }))
            .await
            .expect_err("should reject stream id passed as thread_id");
        let msg = err.message.to_string();
        assert!(msg.contains("create_task"), "tool name missing: {msg}");
        assert!(msg.contains("thread_id"), "param name missing: {msg}");
        assert!(msg.contains("s-deadbeef"), "value missing: {msg}");
        assert!(msg.contains("stream id"), "actual kind missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[tokio::test]
    async fn create_task_rejects_unrecognised_thread_id() {
        let (_proj, _svc, server) = boot();
        let err = server
            .create_task(Parameters(CreateTaskMcpParams {
                thread_id: Some("nonsense".into()),
                backlog: false,
                title: "x".into(),
                description: None,
                description_executive: None,
                description_caveman: None,
                kind: None,
                priority: None,
                status: None,
                parent_id: None,
                touched_files: None,
            }))
            .await
            .expect_err("should reject unprefixed value");
        let msg = err.message.to_string();
        assert!(msg.contains("nonsense"), "value missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[tokio::test]
    async fn upsert_task_round_trips() {
        let (_proj, _services, server) = boot();
        let item = make_task(None, "via mcp");
        let json = serde_json::to_string(&item).unwrap();

        let r = server
            .upsert_task(Parameters(UpsertTaskParams { item_json: json }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("via mcp"), "upsert response: {body}");
        // Parse the response to learn the assigned id, then re-fetch.
        let stored: Task = serde_json::from_str(&body).expect("upsert returns task json");
        assert_ne!(stored.id.value(), 0, "insert must assign a non-zero id");

        let fetched = server
            .get_task(Parameters(TaskIdParams {
                id: stored.id.to_string(),
            }))
            .await
            .unwrap();
        let body = text_payload(fetched);
        assert!(body.contains("via mcp"), "fetched after upsert: {body}");
    }

    #[tokio::test]
    async fn list_wiki_pages_runs_against_empty_store() {
        let (_proj, _services, server) = boot();
        // No notes seeded — the tool should still respond with an
        // empty-list payload rather than erroring.
        let r = server.list_wiki_pages().await.unwrap();
        let body = text_payload(r);
        assert_eq!(body.trim(), "[]");
    }

    #[tokio::test]
    async fn list_stale_wiki_pages_empty_when_nothing_stale() {
        let (_proj, _services, server) = boot();
        // No snapshots / refs seeded — no page can be stale, so the
        // tool returns an empty array rather than erroring.
        let r = server.list_stale_wiki_pages().await.unwrap();
        let body = text_payload(r);
        assert_eq!(body.trim(), "[]");
    }

    // ---- Pure helpers: parse_status / parse_priority / parse_link_type ----

    #[test]
    fn parse_status_accepts_every_status() {
        assert!(matches!(parse_status("ready"), Ok(TaskStatus::Ready)));
        assert!(matches!(
            parse_status("in_progress"),
            Ok(TaskStatus::InProgress)
        ));
        assert!(matches!(parse_status("blocked"), Ok(TaskStatus::Blocked)));
        assert!(matches!(parse_status("done"), Ok(TaskStatus::Done)));
        assert!(matches!(parse_status("canceled"), Ok(TaskStatus::Canceled)));
        assert!(matches!(parse_status("archived"), Ok(TaskStatus::Archived)));
    }

    #[test]
    fn parse_status_rejects_in_progress_with_dash() {
        // The contract says snake_case `in_progress`; clients writing
        // `in-progress` should get an actionable error rather than
        // being silently coerced.
        let err = parse_status("in-progress").unwrap_err();
        assert!(err.message.contains("in-progress"));
    }

    #[test]
    fn parse_priority_accepts_each_value() {
        use oxplow_domain::TaskPriority as P;
        assert!(matches!(parse_priority("low"), Ok(P::Low)));
        assert!(matches!(parse_priority("medium"), Ok(P::Medium)));
        assert!(matches!(parse_priority("high"), Ok(P::High)));
        assert!(matches!(parse_priority("urgent"), Ok(P::Urgent)));
    }

    #[test]
    fn parse_priority_unknown_errors() {
        let err = parse_priority("critical").unwrap_err();
        assert!(err.message.contains("critical"));
    }

    #[test]
    fn parse_link_type_accepts_every_relation() {
        use oxplow_domain::TaskLinkType as L;
        assert!(matches!(parse_link_type("blocks"), Ok(L::Blocks)));
        assert!(matches!(parse_link_type("relates_to"), Ok(L::RelatesTo)));
        assert!(matches!(
            parse_link_type("discovered_from"),
            Ok(L::DiscoveredFrom)
        ));
        assert!(matches!(parse_link_type("duplicates"), Ok(L::Duplicates)));
        assert!(matches!(parse_link_type("supersedes"), Ok(L::Supersedes)));
        assert!(matches!(parse_link_type("replies_to"), Ok(L::RepliesTo)));
    }

    #[test]
    fn parse_link_type_unknown_errors() {
        let err = parse_link_type("flubs").unwrap_err();
        assert!(err.message.contains("flubs"));
    }

    // ---- expect_id_kind ----

    #[test]
    fn expect_id_kind_accepts_matching_prefix() {
        assert!(expect_id_kind("tool", "thread_id", "b-abc123", ID_THREAD,).is_ok());
    }

    #[test]
    fn expect_id_kind_error_names_tool_param_value_and_kinds() {
        let err = expect_id_kind("create_task", "thread_id", "s-abc123", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("create_task"), "tool name missing: {msg}");
        assert!(msg.contains("thread_id"), "param name missing: {msg}");
        assert!(msg.contains("s-abc123"), "value missing: {msg}");
        assert!(msg.contains("stream id"), "actual label missing: {msg}");
        assert!(msg.contains("thread id"), "expected label missing: {msg}");
    }

    #[test]
    fn expect_id_kind_unrecognised_id_shape_errors() {
        // No `<prefix>-…` shape at all — should still be flagged.
        let err = expect_id_kind("tool", "id", "no-prefix-shape", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("no-prefix-shape"), "value missing: {msg}");
    }

    // ---- compose_delegate_query_prompt ----

    #[test]
    fn delegate_query_prompt_contains_required_sections() {
        let s = compose_delegate_query_prompt("b-1", "Where is X?", "", "n-2");
        assert!(s.contains("threadId: b-1"));
        assert!(s.contains("noteId: n-2"));
        assert!(s.contains("## Question"));
        assert!(s.contains("Where is X?"));
        assert!(s.contains("record_query_finding"));
    }

    #[test]
    fn delegate_query_prompt_omits_focus_section_when_empty() {
        let s = compose_delegate_query_prompt("b-1", "Q", "", "n-1");
        assert!(!s.contains("## Focus"));
    }

    #[test]
    fn delegate_query_prompt_includes_focus_when_provided() {
        let s = compose_delegate_query_prompt("b-1", "Q", "look in src/foo.rs", "n-1");
        assert!(s.contains("## Focus"));
        assert!(s.contains("look in src/foo.rs"));
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
}
