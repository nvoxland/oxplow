import type {
  AgentTurn,
  BacklogState,
  EffortDetail,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
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
  CommitPoint,
  WaitPoint,
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
  AgentTurn,
  BacklogState,
  EffortDetail,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
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
  CommitPoint,
  WaitPoint,
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
  getWorkspaceContext(): Promise<WorkspaceContext>;
  createStream(input:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
  ): Promise<Stream>;
  getThreadState(streamId: string): Promise<ThreadState>;
  createThread(streamId: string, title: string): Promise<ThreadState>;
  reorderThread(streamId: string, threadId: string, targetIndex: number): Promise<ThreadState>;
  reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void>;
  reorderStreams(orderedStreamIds: string[]): Promise<void>;
  selectThread(streamId: string, threadId: string): Promise<ThreadState>;
  promoteThread(streamId: string, threadId: string): Promise<ThreadState>;
  completeThread(streamId: string, threadId: string): Promise<ThreadState>;
  renameThread(streamId: string, threadId: string, title: string): Promise<Thread>;
  setAutoCommit(streamId: string, threadId: string, enabled: boolean): Promise<Thread[]>;
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
  listAgentTurns(streamId: string, threadId: string, limit?: number): Promise<AgentTurn[]>;
  /** Open agent turns for a thread — `ended_at IS NULL` AND started after
   *  runtime startup. Feeds the Work panel's in_progress bucket with live
   *  turn rows that disappear when the turn closes. */
  listOpenTurns(threadId: string): Promise<AgentTurn[]>;
  /** Recent closed turns for a thread with `produced_activity = 0` —
   *  feeds the Work panel's "Recent answers" section so Q&A turns stay
   *  re-readable without cluttering Done. */
  listRecentInactiveTurns(threadId: string, limit?: number): Promise<AgentTurn[]>;
  /** Archive a closed turn so it drops out of the Recent-answers list. */
  archiveAgentTurn(turnId: string): Promise<AgentTurn | null>;
  listWorkItemEfforts(itemId: string): Promise<EffortDetail[]>;
  listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]>;
  getSnapshotSummary(snapshotId: string, previousSnapshotId?: string | null): Promise<SnapshotSummary | null>;
  getSnapshotPairDiff(beforeSnapshotId: string | null, afterSnapshotId: string, path: string): Promise<SnapshotDiffResult>;
  getEffortFiles(effortId: string): Promise<SnapshotSummary | null>;
  listEffortsEndingAtSnapshots(snapshotIds: string[]): Promise<Record<string, Array<{ effortId: string; workItemId: string; threadId: string; title: string; status: WorkItemStatus; priority: WorkItemPriority }>>>;
  restoreFileFromSnapshot(streamId: string, snapshotId: string, path: string): Promise<void>;
  getBranchChanges(streamId: string, baseRef?: string): Promise<BranchChanges & { resolvedBaseRef: string | null }>;
  getGitLog(streamId: string, options?: { limit?: number }): Promise<GitLogResult>;
  getCommitDetail(streamId: string, sha: string): Promise<CommitDetail | null>;
  getChangeScopes(streamId: string): Promise<ChangeScopes>;
  searchWorkspaceText(streamId: string, query: string, options?: { limit?: number }): Promise<TextSearchHit[]>;
  gitRestorePath(streamId: string, path: string): Promise<GitOpResult>;
  gitAddPath(streamId: string, path: string): Promise<GitOpResult>;
  gitAppendToGitignore(streamId: string, path: string): Promise<GitOpResult>;
  gitPush(streamId: string, options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string }): Promise<GitOpResult>;
  gitPull(streamId: string, options?: { rebase?: boolean; remote?: string; branch?: string }): Promise<GitOpResult>;
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
  listCommitPoints(threadId: string): Promise<CommitPoint[]>;
  createCommitPoint(streamId: string, threadId: string): Promise<CommitPoint>;
  deleteCommitPoint(id: string): Promise<void>;
  updateCommitPoint(id: string, changes: { mode?: "auto" | "approve" }): Promise<CommitPoint[]>;
  commitCommitPoint(id: string, message: string): Promise<CommitPoint>;
  reorderThreadQueue(streamId: string, threadId: string, entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>): Promise<void>;
  listWaitPoints(threadId: string): Promise<WaitPoint[]>;
  createWaitPoint(streamId: string, threadId: string, note?: string | null): Promise<WaitPoint>;
  setWaitPointNote(id: string, note: string | null): Promise<WaitPoint>;
  deleteWaitPoint(id: string): Promise<void>;
  listHookEvents(streamId?: string): Promise<StoredEvent[]>;
  listAgentStatuses(streamId?: string): Promise<Array<{ streamId: string; threadId: string; status: AgentStatus }>>;
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
