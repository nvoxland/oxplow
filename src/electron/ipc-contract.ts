import type {
  AgentTurn,
  BacklogState,
  BatchFileChange,
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
  CommitPointMode,
  WaitPoint,
  RefOption,
  BatchWorkState,
  Stream,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
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
  CommitPointMode,
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
  listBranches(): Promise<BranchRef[]>;
  getWorkspaceContext(): Promise<WorkspaceContext>;
  createStream(input:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
  ): Promise<Stream>;
  getBatchState(streamId: string): Promise<BatchState>;
  createBatch(streamId: string, title: string): Promise<BatchState>;
  reorderBatch(streamId: string, batchId: string, targetIndex: number): Promise<BatchState>;
  selectBatch(streamId: string, batchId: string): Promise<BatchState>;
  promoteBatch(streamId: string, batchId: string): Promise<BatchState>;
  completeBatch(streamId: string, batchId: string): Promise<BatchState>;
  renameBatch(streamId: string, batchId: string, title: string): Promise<Batch>;
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
  moveWorkItemToBatch(streamId: string, fromBatchId: string, itemId: string, toBatchId: string): Promise<{ from: BatchWorkState; to: BatchWorkState }>;
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
  listAgentTurns(streamId: string, batchId: string, limit?: number): Promise<AgentTurn[]>;
  listBatchFileChanges(streamId: string, batchId: string, limit?: number): Promise<BatchFileChange[]>;
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
  createCommitPoint(streamId: string, batchId: string, mode: CommitPointMode): Promise<CommitPoint>;
  setCommitPointMode(id: string, mode: CommitPointMode): Promise<CommitPoint>;
  approveCommitPoint(id: string, editedMessage?: string): Promise<CommitPoint>;
  rejectCommitPoint(id: string, note: string): Promise<CommitPoint>;
  resetCommitPoint(id: string): Promise<CommitPoint>;
  deleteCommitPoint(id: string): Promise<void>;
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
