import type {
  BacklogState,
  EffortDetail,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
  GroupedGitRefs,
  Thread,
  ThreadState,
  GitFileStatus,
  GitLogResult,
  CommitDetail,
  ChangeScopes,
  TextSearchHit,
  GitOpResult,
  GitLogCommit,
  BlameLine,
  RefOption,
  ThreadWorkState,
  Stream,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkNote,
  WorkspaceContext,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceIndexedFile,
  WorkspacePathChange,
  WorkspaceRenameResult,
  WorkspaceStatusSummary,
  WorkspaceWatchEvent,
  StoredEvent,
} from "../ui/api.js";
import type { AgentStatus, OxplowEvent } from "../core/event-bus.js";
import type { CommandId, MenuGroupSnapshot } from "../ui/commands.js";

export type {
  BacklogState,
  EffortDetail,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
  GroupedGitRefs,
  Thread,
  ThreadState,
  ThreadWorkState,
  CommandId,
  GitFileStatus,
  GitLogResult,
  CommitDetail,
  ChangeScopes,
  TextSearchHit,
  GitOpResult,
  GitLogCommit,
  BlameLine,
  RefOption,
  AgentStatus,
  MenuGroupSnapshot,
  OxplowEvent,
  Stream,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkNote,
  WorkspaceContext,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceIndexedFile,
  WorkspacePathChange,
  WorkspaceRenameResult,
  WorkspaceStatusSummary,
  WorkspaceWatchEvent,
  StoredEvent,
};

export interface WikiNoteSearchHit {
  slug: string;
  title: string;
  /** Snippet with `<mark>…</mark>` highlights around the matched terms. */
  snippet: string;
  updated_at: string;
}

export interface UsageRollup {
  key: string;
  last_at: string;
  count: number;
}

export type CodeQualityTool = "lizard" | "jscpd";
export type CodeQualityScope = "codebase" | "diff";
export type CodeQualityScanStatus = "running" | "completed" | "failed";
export type CodeQualityFindingKind =
  | "complexity"
  | "function-length"
  | "parameter-count"
  | "duplicate-block";

export interface CodeQualityScanRow {
  id: number;
  stream_id: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  base_ref: string | null;
  status: CodeQualityScanStatus;
  error_message: string | null;
  started_at: string;
  completed_at: string | null;
}

export interface CodeQualityFindingRow {
  id: number;
  scanId: number;
  path: string;
  startLine: number;
  endLine: number;
  kind: CodeQualityFindingKind;
  metricValue: number;
  extra: Record<string, unknown> | null;
}

export interface WikiNoteSummary {
  slug: string;
  title: string;
  updated_at: string;
  created_at: string;
  freshness: "fresh" | "stale" | "very-stale";
  head_advanced: boolean;
  changed_refs: string[];
  deleted_refs: string[];
  total_refs: number;
  /** Workspace-relative paths the watcher parsed from the note body
   *  (the same set that backs `total_refs` and freshness). Used by the
   *  UI to render a clickable backlinks affordance. */
  referenced_files: string[];
}

export interface UiLogPayload {
  clientId: string;
  level: "debug" | "info" | "warn" | "error";
  message: string;
  context?: Record<string, unknown>;
  timestamp?: string;
}

export interface TerminalEvent {
  sessionId: string;
  message: string;
}

export interface LspEvent {
  clientId: string;
  message: string;
}

export interface EditorFocusPayload {
  streamId: string;
  activeFile: string | null;
  caret: { line: number; column: number } | null;
  selection: {
    startLine: number;
    startColumn: number;
    endLine: number;
    endColumn: number;
    text: string;
  } | null;
  openFiles: { path: string; dirty: boolean }[];
}

export interface DesktopApi {
  getCurrentStream(): Promise<Stream>;
  listStreams(): Promise<Stream[]>;
  switchStream(id: string): Promise<Stream>;
  renameCurrentStream(title: string): Promise<Stream>;
  renameStream(streamId: string, title: string): Promise<Stream>;
  getConfig(): Promise<import("../config/config.js").OxplowConfig>;
  setAgentPromptAppend(text: string): Promise<import("../config/config.js").OxplowConfig>;
  setSnapshotRetentionDays(days: number): Promise<import("../config/config.js").OxplowConfig>;
  setSnapshotMaxFileBytes(bytes: number): Promise<import("../config/config.js").OxplowConfig>;
  setGeneratedDirs(dirs: string[]): Promise<import("../config/config.js").OxplowConfig>;
  listBranches(): Promise<BranchRef[]>;
  getDefaultBranch(): Promise<string | null>;
  clipboardReadText(): Promise<string>;
  listGitRefs(): Promise<GroupedGitRefs>;
  renameGitBranch(from: string, to: string): Promise<GitOpResult>;
  deleteGitBranch(branch: string, options?: { force?: boolean }): Promise<GitOpResult>;
  gitMergeInto(streamId: string, other: string): Promise<GitOpResult>;
  gitRebaseOnto(streamId: string, onto: string): Promise<GitOpResult>;
  getWorkspaceContext(): Promise<WorkspaceContext>;
  createStream(input:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string }
    | { title: string; summary?: string; source: "worktree"; worktreePath: string },
  ): Promise<Stream>;
  listAdoptableWorktrees(): Promise<import("../git/git.js").GitWorktreeEntry[]>;
  listSiblingWorktrees(streamId: string): Promise<import("../git/git.js").GitWorktreeEntry[]>;
  checkoutStreamBranch(streamId: string, branch: string): Promise<Stream>;
  getThreadState(streamId: string): Promise<ThreadState>;
  createThread(streamId: string, title: string): Promise<ThreadState>;
  reorderThread(streamId: string, threadId: string, targetIndex: number): Promise<ThreadState>;
  reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void>;
  reorderStreams(orderedStreamIds: string[]): Promise<void>;
  selectThread(streamId: string, threadId: string): Promise<ThreadState>;
  promoteThread(streamId: string, threadId: string): Promise<ThreadState>;
  closeThread(streamId: string, threadId: string): Promise<ThreadState>;
  reopenThread(streamId: string, threadId: string): Promise<ThreadState>;
  listClosedThreads(streamId: string): Promise<Thread[]>;
  renameThread(streamId: string, threadId: string, title: string): Promise<Thread>;
  setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]>;
  setThreadPrompt(streamId: string, threadId: string, prompt: string | null): Promise<Thread[]>;
  getThreadWorkState(streamId: string, threadId: string): Promise<ThreadWorkState>;
  createWorkItem(
    streamId: string,
    threadId: string,
    input: {
      kind: WorkItemKind;
      title: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<ThreadWorkState>;
  updateWorkItem(
    streamId: string,
    threadId: string,
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<ThreadWorkState>;
  deleteWorkItem(streamId: string, threadId: string, itemId: string): Promise<ThreadWorkState>;
  reorderWorkItems(streamId: string, threadId: string, orderedItemIds: string[]): Promise<ThreadWorkState>;
  moveWorkItemToThread(streamId: string, fromThreadId: string, itemId: string, toThreadId: string, toStreamId?: string): Promise<{ from: ThreadWorkState; to: ThreadWorkState }>;
  getBacklogState(): Promise<BacklogState>;
  createBacklogItem(input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  }): Promise<BacklogState>;
  updateBacklogItem(
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      acceptanceCriteria?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<BacklogState>;
  deleteBacklogItem(itemId: string): Promise<BacklogState>;
  reorderBacklog(orderedItemIds: string[]): Promise<BacklogState>;
  moveWorkItemToBacklog(streamId: string, fromThreadId: string, itemId: string): Promise<{ from: ThreadWorkState; backlog: BacklogState }>;
  moveBacklogItemToThread(streamId: string, itemId: string, toThreadId: string): Promise<{ backlog: BacklogState; to: ThreadWorkState }>;
  addWorkItemNote(streamId: string, threadId: string, itemId: string, note: string): Promise<WorkItemEvent[]>;
  listWorkItemEvents(streamId: string, threadId: string, itemId?: string): Promise<WorkItemEvent[]>;
  getWorkNotes(itemId: string): Promise<WorkNote[]>;
  listWorkItemEfforts(itemId: string): Promise<EffortDetail[]>;
  listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]>;
  getSnapshotSummary(snapshotId: string, previousSnapshotId?: string | null): Promise<SnapshotSummary | null>;
  getSnapshotPairDiff(beforeSnapshotId: string | null, afterSnapshotId: string, path: string): Promise<SnapshotDiffResult>;
  getEffortFiles(effortId: string): Promise<SnapshotSummary | null>;
  listEffortsEndingAtSnapshots(snapshotIds: string[]): Promise<Record<string, Array<{ effortId: string; workItemId: string; threadId: string; title: string; status: WorkItemStatus; priority: WorkItemPriority }>>>;
  restoreFileFromSnapshot(streamId: string, snapshotId: string, path: string): Promise<void>;
  getBranchChanges(streamId: string, baseRef?: string): Promise<BranchChanges & { resolvedBaseRef: string | null }>;
  getGitLog(streamId: string, options?: { limit?: number; all?: boolean }): Promise<GitLogResult>;
  getCommitDetail(streamId: string, sha: string): Promise<CommitDetail | null>;
  getChangeScopes(streamId: string): Promise<ChangeScopes>;
  searchWorkspaceText(streamId: string, query: string, options?: { limit?: number }): Promise<TextSearchHit[]>;
  gitRestorePath(streamId: string, path: string): Promise<GitOpResult>;
  gitAddPath(streamId: string, path: string): Promise<GitOpResult>;
  gitAppendToGitignore(streamId: string, path: string): Promise<GitOpResult>;
  gitPush(streamId: string, options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string }): Promise<GitOpResult>;
  gitPull(streamId: string, options?: { rebase?: boolean; remote?: string; branch?: string }): Promise<GitOpResult>;
  gitFetch(streamId: string, options?: { remote?: string; prune?: boolean; all?: boolean }): Promise<GitOpResult>;
  getAheadBehind(streamId: string, base: string, head?: string): Promise<{ ahead: number; behind: number }>;
  getCommitsAheadOf(streamId: string, base: string, head: string, limit?: number): Promise<GitLogCommit[]>;
  listRecentRemoteBranches(streamId: string, limit?: number): Promise<import("../git/git.js").RemoteBranchEntry[]>;
  gitPushCurrentTo(streamId: string, remote: string, branch: string): Promise<GitOpResult>;
  gitPullRemoteIntoCurrent(streamId: string, remote: string, branch: string): Promise<GitOpResult>;
  gitCommitAll(streamId: string, message: string, options?: { includeUntracked?: boolean }): Promise<GitOpResult & { sha?: string }>;
  listFileCommits(streamId: string, path: string, limit?: number): Promise<GitLogCommit[]>;
  gitBlame(streamId: string, path: string): Promise<BlameLine[]>;
  localBlame(streamId: string, path: string): Promise<import("./local-blame.js").LocalBlameEntry[]>;
  listAllRefs(streamId: string): Promise<RefOption[]>;
  readFileAtRef(streamId: string, ref: string, path: string): Promise<{ content: string | null }>;
  listWorkspaceEntries(streamId: string, path?: string): Promise<WorkspaceEntry[]>;
  listWorkspaceFiles(streamId: string): Promise<{ files: WorkspaceIndexedFile[]; summary: WorkspaceStatusSummary }>;
  readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile>;
  writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile>;
  createWorkspaceFile(streamId: string, path: string, content?: string): Promise<WorkspaceFile>;
  createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange>;
  renameWorkspacePath(streamId: string, fromPath: string, toPath: string): Promise<WorkspaceRenameResult>;
  deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange>;
  listWikiNotes(streamId: string): Promise<WikiNoteSummary[]>;
  readWikiNoteBody(streamId: string, slug: string): Promise<string>;
  writeWikiNoteBody(streamId: string, slug: string, body: string): Promise<void>;
  deleteWikiNote(streamId: string, slug: string): Promise<void>;
  searchWikiNotes(streamId: string, query: string, limit?: number): Promise<WikiNoteSearchHit[]>;
  recordUsage(input: { kind: string; key: string; event?: string; streamId?: string | null; threadId?: string | null }): Promise<void>;
  listRecentUsage(input: { kind: string; streamId?: string | null; threadId?: string | null; limit?: number; since?: string }): Promise<UsageRollup[]>;
  listFrequentUsage(input: { kind: string; streamId?: string | null; threadId?: string | null; limit?: number; since?: string }): Promise<UsageRollup[]>;
  listCurrentlyOpenUsage(input: { kind: string; streamId?: string | null; threadId?: string | null }): Promise<string[]>;
  runCodeQualityScan(input: { streamId: string; tool: CodeQualityTool; scope: CodeQualityScope; baseRef?: string | null }): Promise<CodeQualityScanRow>;
  listCodeQualityFindings(input: { streamId: string; tool?: CodeQualityTool; paths?: string[] }): Promise<CodeQualityFindingRow[]>;
  listCodeQualityScans(input: { streamId: string; limit?: number }): Promise<CodeQualityScanRow[]>;
  getWorkItemSummaries(ids: string[]): Promise<Array<{ id: string; title: string; status: WorkItemStatus; thread_id: string | null }>>;
  reorderThreadQueue(streamId: string, threadId: string, entries: Array<{ id: string }>): Promise<void>;
  /** Drop a transient agent follow-up reminder. The store is in-memory
   *  on the runtime — adds happen via MCP tool calls; the UI only ever
   *  removes (the × button on each reminder line). */
  removeFollowup(threadId: string, id: string): Promise<void>;
  /** Snapshot of currently-running and recently-finished background
   *  tasks (git ops, code-quality scans, LSP startup, notes resync).
   *  Subscribe to `background-task.changed` events and refetch. */
  listBackgroundTasks(): Promise<import("./background-task-store.js").BackgroundTask[]>;
  listHookEvents(streamId?: string): Promise<StoredEvent[]>;
  listAgentStatuses(streamId?: string): Promise<Array<{ streamId: string; threadId: string; status: AgentStatus }>>;
  listRecentlyFinished(threadId: string | null, limit: number): Promise<
    Array<
      | { kind: "work-item"; itemId: string; title: string; t: string }
      | { kind: "note"; slug: string; title: string; t: string }
    >
  >;
  clearRecentlyFinished(threadId: string | null): Promise<void>;
  recordPageVisit(input: import("../persistence/page-visit-store.js").PageVisitInput): Promise<void>;
  listRecentPageVisits(opts: import("../persistence/page-visit-store.js").ListRecentOpts):
    Promise<import("../persistence/page-visit-store.js").PageVisit[]>;
  topVisitedPages(opts: import("../persistence/page-visit-store.js").TopVisitedOpts):
    Promise<import("../persistence/page-visit-store.js").TopVisitedRow[]>;
  countPageVisitsByDay(opts: import("../persistence/page-visit-store.js").CountByDayOpts):
    Promise<import("../persistence/page-visit-store.js").CountByDayRow[]>;
  ping(): Promise<boolean>;
  logUi(payload: UiLogPayload): Promise<void>;
  updateEditorFocus(payload: EditorFocusPayload): Promise<void>;
  setNativeMenu(groups: MenuGroupSnapshot[]): Promise<void>;
  openTerminalSession(paneTarget: string, cols: number, rows: number, mode?: "direct" | "tmux"): Promise<string>;
  sendTerminalMessage(sessionId: string, message: string): Promise<void>;
  closeTerminalSession(sessionId: string): Promise<void>;
  openLspClient(streamId: string, languageId: string): Promise<string>;
  sendLspMessage(clientId: string, message: string): Promise<void>;
  closeLspClient(clientId: string): Promise<void>;
  onOxplowEvent(listener: (event: OxplowEvent) => void): () => void;
  onTerminalEvent(listener: (event: TerminalEvent) => void): () => void;
  onLspEvent(listener: (event: LspEvent) => void): () => void;
  onMenuCommand(listener: (commandId: CommandId) => void): () => void;
}
