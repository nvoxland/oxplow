import type {
  AgentTurn,
  BacklogState,
  BatchFileChange,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
  Batch,
  BatchState,
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
  BatchWorkState,
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
import type { AgentStatus, NewdeEvent } from "../core/event-bus.js";
import type { CommandId, MenuGroupSnapshot } from "../ui/commands.js";

export type {
  AgentTurn,
  BacklogState,
  BatchFileChange,
  FileSnapshot,
  SnapshotDiffResult,
  SnapshotSummary,
  BranchChanges,
  BranchRef,
  Batch,
  BatchState,
  BatchWorkState,
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
  NewdeEvent,
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
  getConfig(): Promise<import("../config/config.js").NewdeConfig>;
  setAgentPromptAppend(text: string): Promise<import("../config/config.js").NewdeConfig>;
  setSnapshotRetentionDays(days: number): Promise<import("../config/config.js").NewdeConfig>;
  setSnapshotMaxFileBytes(bytes: number): Promise<import("../config/config.js").NewdeConfig>;
  setGeneratedDirs(dirs: string[]): Promise<import("../config/config.js").NewdeConfig>;
  listBranches(): Promise<BranchRef[]>;
  getWorkspaceContext(): Promise<WorkspaceContext>;
  createStream(input:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
  ): Promise<Stream>;
  getBatchState(streamId: string): Promise<BatchState>;
  createBatch(streamId: string, title: string): Promise<BatchState>;
  reorderBatch(streamId: string, batchId: string, targetIndex: number): Promise<BatchState>;
  reorderBatches(streamId: string, orderedBatchIds: string[]): Promise<void>;
  reorderStreams(orderedStreamIds: string[]): Promise<void>;
  selectBatch(streamId: string, batchId: string): Promise<BatchState>;
  promoteBatch(streamId: string, batchId: string): Promise<BatchState>;
  completeBatch(streamId: string, batchId: string): Promise<BatchState>;
  renameBatch(streamId: string, batchId: string, title: string): Promise<Batch>;
  setAutoCommit(streamId: string, batchId: string, enabled: boolean): Promise<Batch[]>;
  setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]>;
  setBatchPrompt(streamId: string, batchId: string, prompt: string | null): Promise<Batch[]>;
  getBatchWorkState(streamId: string, batchId: string): Promise<BatchWorkState>;
  createWorkItem(
    streamId: string,
    batchId: string,
    input: {
      kind: WorkItemKind;
      title: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<BatchWorkState>;
  updateWorkItem(
    streamId: string,
    batchId: string,
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      acceptanceCriteria?: string | null;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): Promise<BatchWorkState>;
  deleteWorkItem(streamId: string, batchId: string, itemId: string): Promise<BatchWorkState>;
  reorderWorkItems(streamId: string, batchId: string, orderedItemIds: string[]): Promise<BatchWorkState>;
  moveWorkItemToBatch(streamId: string, fromBatchId: string, itemId: string, toBatchId: string, toStreamId?: string): Promise<{ from: BatchWorkState; to: BatchWorkState }>;
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
  moveWorkItemToBacklog(streamId: string, fromBatchId: string, itemId: string): Promise<{ from: BatchWorkState; backlog: BacklogState }>;
  moveBacklogItemToBatch(streamId: string, itemId: string, toBatchId: string): Promise<{ backlog: BacklogState; to: BatchWorkState }>;
  addWorkItemNote(streamId: string, batchId: string, itemId: string, note: string): Promise<WorkItemEvent[]>;
  listWorkItemEvents(streamId: string, batchId: string, itemId?: string): Promise<WorkItemEvent[]>;
  getWorkNotes(itemId: string): Promise<WorkNote[]>;
  listAgentTurns(streamId: string, batchId: string, limit?: number): Promise<AgentTurn[]>;
  listBatchFileChanges(streamId: string, batchId: string, limit?: number): Promise<BatchFileChange[]>;
  getTurnFileDiff(turnId: string, path: string): Promise<SnapshotDiffResult>;
  listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]>;
  getSnapshotSummary(snapshotId: string): Promise<SnapshotSummary | null>;
  getSnapshotFileDiff(snapshotId: string, path: string): Promise<SnapshotDiffResult>;
  getSnapshotPairDiff(beforeSnapshotId: string | null, afterSnapshotId: string, path: string): Promise<SnapshotDiffResult>;
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
  listCommitPoints(batchId: string): Promise<CommitPoint[]>;
  createCommitPoint(streamId: string, batchId: string): Promise<CommitPoint>;
  deleteCommitPoint(id: string): Promise<void>;
  updateCommitPoint(id: string, changes: { mode?: "auto" | "approve" }): Promise<CommitPoint[]>;
  commitCommitPoint(id: string, message: string): Promise<CommitPoint>;
  reorderBatchQueue(streamId: string, batchId: string, entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>): Promise<void>;
  listWaitPoints(batchId: string): Promise<WaitPoint[]>;
  createWaitPoint(streamId: string, batchId: string, note?: string | null): Promise<WaitPoint>;
  setWaitPointNote(id: string, note: string | null): Promise<WaitPoint>;
  deleteWaitPoint(id: string): Promise<void>;
  listHookEvents(streamId?: string): Promise<StoredEvent[]>;
  listAgentStatuses(streamId?: string): Promise<Array<{ streamId: string; batchId: string; status: AgentStatus }>>;
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
  onNewdeEvent(listener: (event: NewdeEvent) => void): () => void;
  onTerminalEvent(listener: (event: TerminalEvent) => void): () => void;
  onLspEvent(listener: (event: LspEvent) => void): () => void;
  onMenuCommand(listener: (commandId: CommandId) => void): () => void;
}
