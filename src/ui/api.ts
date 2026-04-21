import type { DesktopApi, NewdeEvent } from "../electron/ipc-contract.js";

export type { NewdeEvent } from "../electron/ipc-contract.js";
export type { GitLogResult, GitLogCommit, GitLogRef, CommitDetail, ChangeScopes, TextSearchHit, GitOpResult, RefOption, BlameLine } from "../git/git.js";
export type { CommitPoint, CommitPointMode, CommitPointStatus } from "../persistence/commit-point-store.js";
export type { WaitPoint, WaitPointStatus } from "../persistence/wait-point-store.js";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  branch_ref: string;
  branch_source: "local" | "remote" | "new";
  worktree_path: string;
  created_at: string;
  updated_at: string;
  custom_prompt: string | null;
  panes: { working: string; talking: string };
  resume: { working_session_id: string; talking_session_id: string };
}

export interface Batch {
  id: string;
  stream_id: string;
  title: string;
  status: "active" | "queued" | "completed";
  sort_index: number;
  created_at: string;
  updated_at: string;
  pane_target: string;
  resume_session_id: string;
  auto_commit: boolean;
  custom_prompt: string | null;
}

export interface BatchState {
  selectedBatchId: string | null;
  activeBatchId: string | null;
  batches: Batch[];
}

export type WorkItemKind = "epic" | "task" | "subtask" | "bug" | "note";
export type WorkItemStatus = "ready" | "in_progress" | "human_check" | "blocked" | "done" | "canceled" | "archived";
export type WorkItemPriority = "low" | "medium" | "high" | "urgent";

export interface WorkItem {
  id: string;
  batch_id: string | null;
  parent_id: string | null;
  kind: WorkItemKind;
  title: string;
  description: string;
  acceptance_criteria: string | null;
  status: WorkItemStatus;
  priority: WorkItemPriority;
  sort_index: number;
  created_by: "user" | "agent" | "system";
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  note_count: number;
}

export interface WorkNote {
  id: string;
  work_item_id: string;
  body: string;
  author: string;
  created_at: string;
}

export interface WorkItemEvent {
  id: string;
  batch_id: string;
  item_id: string | null;
  event_type: string;
  actor_kind: "user" | "agent" | "system";
  actor_id: string;
  payload_json: string;
  created_at: string;
}

export type FileChangeKind = "created" | "updated" | "deleted";
export type FileChangeSource = "hook" | "fs-watch";

export interface BatchFileChange {
  id: string;
  batch_id: string;
  turn_id: string | null;
  work_item_id: string | null;
  path: string;
  change_kind: FileChangeKind;
  source: FileChangeSource;
  tool_name: string | null;
  created_at: string;
}

export type SnapshotKind = "turn-start" | "turn-end";

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  kind: SnapshotKind;
  turn_id: string | null;
  batch_id: string | null;
  parent_snapshot_id: string | null;
  created_at: string;
  turn_prompt: string | null;
}

export type SnapshotEntryState = "present" | "deleted" | "oversize";

export interface SnapshotEntry {
  hash: string;
  mtime_ms: number;
  size: number;
  state: SnapshotEntryState;
}

export interface SnapshotFileRow {
  entry: SnapshotEntry;
  kind: "created" | "updated" | "deleted";
}

export interface SnapshotSummary {
  snapshot: FileSnapshot;
  files: Record<string, SnapshotFileRow>;
  counts: { created: number; updated: number; deleted: number };
}

export type SnapshotDiffSide = "absent" | SnapshotEntryState;

export interface SnapshotDiffResult {
  before: string | null;
  after: string | null;
  beforeState: SnapshotDiffSide;
  afterState: SnapshotDiffSide;
}

export interface AgentTurn {
  id: string;
  batch_id: string;
  work_item_id: string | null;
  prompt: string;
  answer: string | null;
  session_id: string | null;
  started_at: string;
  ended_at: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_input_tokens: number | null;
}

export interface BatchWorkState {
  batchId: string;
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
  epics: WorkItem[];
  items: WorkItem[];
}

export interface BacklogState {
  items: WorkItem[];
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
}

export const BACKLOG_SCOPE = "__backlog__";

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

export interface BranchChangeEntry {
  path: string;
  status: GitFileStatus;
  additions: number | null;
  deletions: number | null;
}

export interface BranchChanges {
  baseRef: string;
  mergeBase: string | null;
  files: BranchChangeEntry[];
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "file" | "directory";
  gitStatus: GitFileStatus | null;
  hasChanges: boolean;
}

export interface WorkspaceFile {
  path: string;
  content: string;
}

export interface WorkspacePathChange {
  path: string;
}

export interface WorkspaceRenameResult {
  fromPath: string;
  toPath: string;
}

export interface WorkspaceIndexedFile {
  path: string;
  gitStatus: GitFileStatus | null;
}

export interface WorkspaceStatusSummary {
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  total: number;
}

export interface WorkspaceContext {
  gitEnabled: boolean;
}

export interface WorkspaceWatchEvent {
  id: number;
  streamId: string;
  path: string;
  kind: "created" | "updated" | "deleted";
  t: number;
}

export async function getCurrentStream(): Promise<Stream> {
  return desktopApi().getCurrentStream();
}

export async function listStreams(): Promise<Stream[]> {
  return desktopApi().listStreams();
}

export async function switchStream(id: string): Promise<Stream> {
  return desktopApi().switchStream(id);
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  return desktopApi().renameCurrentStream(title);
}

export async function renameStream(streamId: string, title: string): Promise<Stream> {
  return desktopApi().renameStream(streamId, title);
}

export async function getConfig(): Promise<import("../config/config.js").NewdeConfig> {
  return desktopApi().getConfig();
}

export async function setAgentPromptAppend(text: string): Promise<import("../config/config.js").NewdeConfig> {
  return desktopApi().setAgentPromptAppend(text);
}

export async function setGeneratedDirs(dirs: string[]): Promise<import("../config/config.js").NewdeConfig> {
  return desktopApi().setGeneratedDirs(dirs);
}

export async function setSnapshotRetentionDays(days: number): Promise<import("../config/config.js").NewdeConfig> {
  return desktopApi().setSnapshotRetentionDays(days);
}

export async function setSnapshotMaxFileBytes(bytes: number): Promise<import("../config/config.js").NewdeConfig> {
  return desktopApi().setSnapshotMaxFileBytes(bytes);
}

export async function listBranches(): Promise<BranchRef[]> {
  return desktopApi().listBranches();
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  return desktopApi().getWorkspaceContext();
}

export async function createStream(input:
  | { title: string; summary?: string; source: "existing"; ref: string }
  | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
): Promise<Stream> {
  return desktopApi().createStream(input);
}

export async function getBatchState(streamId: string): Promise<BatchState> {
  return desktopApi().getBatchState(streamId);
}

export async function createBatch(streamId: string, title: string): Promise<BatchState> {
  return desktopApi().createBatch(streamId, title);
}

export async function reorderBatch(streamId: string, batchId: string, targetIndex: number): Promise<BatchState> {
  return desktopApi().reorderBatch(streamId, batchId, targetIndex);
}

export async function reorderBatches(streamId: string, orderedBatchIds: string[]): Promise<void> {
  return desktopApi().reorderBatches(streamId, orderedBatchIds);
}

export async function reorderStreams(orderedStreamIds: string[]): Promise<void> {
  return desktopApi().reorderStreams(orderedStreamIds);
}

export async function selectBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().selectBatch(streamId, batchId);
}

export async function promoteBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().promoteBatch(streamId, batchId);
}

export async function completeBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().completeBatch(streamId, batchId);
}

export async function renameBatch(streamId: string, batchId: string, title: string): Promise<Batch> {
  return desktopApi().renameBatch(streamId, batchId, title);
}

export async function setAutoCommit(streamId: string, batchId: string, enabled: boolean): Promise<Batch[]> {
  return desktopApi().setAutoCommit(streamId, batchId, enabled);
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  return desktopApi().setStreamPrompt(streamId, prompt);
}

export async function setBatchPrompt(streamId: string, batchId: string, prompt: string | null): Promise<Batch[]> {
  return desktopApi().setBatchPrompt(streamId, batchId, prompt);
}

export async function getBatchWorkState(streamId: string, batchId: string): Promise<BatchWorkState> {
  return desktopApi().getBatchWorkState(streamId, batchId);
}

export async function createWorkItem(
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
): Promise<BatchWorkState> {
  return desktopApi().createWorkItem(streamId, batchId, input);
}

export async function updateWorkItem(
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
): Promise<BatchWorkState> {
  return desktopApi().updateWorkItem(streamId, batchId, itemId, changes);
}

export async function deleteWorkItem(
  streamId: string,
  batchId: string,
  itemId: string,
): Promise<BatchWorkState> {
  return desktopApi().deleteWorkItem(streamId, batchId, itemId);
}

export async function reorderWorkItems(
  streamId: string,
  batchId: string,
  orderedItemIds: string[],
): Promise<BatchWorkState> {
  return desktopApi().reorderWorkItems(streamId, batchId, orderedItemIds);
}

export async function moveWorkItemToBatch(
  streamId: string,
  fromBatchId: string,
  itemId: string,
  toBatchId: string,
  toStreamId?: string,
): Promise<{ from: BatchWorkState; to: BatchWorkState }> {
  return desktopApi().moveWorkItemToBatch(streamId, fromBatchId, itemId, toBatchId, toStreamId);
}

export async function getBacklogState(): Promise<BacklogState> {
  return desktopApi().getBacklogState();
}

export async function createBacklogItem(input: {
  kind: WorkItemKind;
  title: string;
  description?: string;
  acceptanceCriteria?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}): Promise<BacklogState> {
  return desktopApi().createBacklogItem(input);
}

export async function updateBacklogItem(
  itemId: string,
  changes: {
    title?: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  },
): Promise<BacklogState> {
  return desktopApi().updateBacklogItem(itemId, changes);
}

export async function deleteBacklogItem(itemId: string): Promise<BacklogState> {
  return desktopApi().deleteBacklogItem(itemId);
}

export async function reorderBacklog(orderedItemIds: string[]): Promise<BacklogState> {
  return desktopApi().reorderBacklog(orderedItemIds);
}

export async function moveWorkItemToBacklog(
  streamId: string,
  fromBatchId: string,
  itemId: string,
): Promise<{ from: BatchWorkState; backlog: BacklogState }> {
  return desktopApi().moveWorkItemToBacklog(streamId, fromBatchId, itemId);
}

export async function moveBacklogItemToBatch(
  streamId: string,
  itemId: string,
  toBatchId: string,
): Promise<{ backlog: BacklogState; to: BatchWorkState }> {
  return desktopApi().moveBacklogItemToBatch(streamId, itemId, toBatchId);
}

export async function getGitLog(
  streamId: string,
  options?: { limit?: number },
): Promise<import("../git/git.js").GitLogResult> {
  return desktopApi().getGitLog(streamId, options);
}

export async function getCommitDetail(
  streamId: string,
  sha: string,
): Promise<import("../git/git.js").CommitDetail | null> {
  return desktopApi().getCommitDetail(streamId, sha);
}

export async function getChangeScopes(
  streamId: string,
): Promise<import("../git/git.js").ChangeScopes> {
  return desktopApi().getChangeScopes(streamId);
}

export async function searchWorkspaceText(
  streamId: string,
  query: string,
  options?: { limit?: number },
): Promise<import("../git/git.js").TextSearchHit[]> {
  return desktopApi().searchWorkspaceText(streamId, query, options);
}

export async function gitRestorePath(streamId: string, path: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitRestorePath(streamId, path);
}

export async function gitAddPath(streamId: string, path: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitAddPath(streamId, path);
}

export async function gitAppendToGitignore(streamId: string, path: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitAppendToGitignore(streamId, path);
}

export async function gitPush(
  streamId: string,
  options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string },
): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitPush(streamId, options);
}

export async function gitPull(
  streamId: string,
  options?: { rebase?: boolean; remote?: string; branch?: string },
): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitPull(streamId, options);
}

export async function gitCommitAll(
  streamId: string,
  message: string,
  options?: { includeUntracked?: boolean },
): Promise<import("../git/git.js").GitOpResult & { sha?: string }> {
  return desktopApi().gitCommitAll(streamId, message, options);
}

export async function listFileCommits(
  streamId: string,
  path: string,
  limit?: number,
): Promise<import("../git/git.js").GitLogCommit[]> {
  return desktopApi().listFileCommits(streamId, path, limit);
}

export async function gitBlame(
  streamId: string,
  path: string,
): Promise<import("../git/git.js").BlameLine[]> {
  return desktopApi().gitBlame(streamId, path);
}

export async function listCommitPoints(batchId: string): Promise<import("../persistence/commit-point-store.js").CommitPoint[]> {
  return desktopApi().listCommitPoints(batchId);
}

export async function createCommitPoint(
  streamId: string,
  batchId: string,
): Promise<import("../persistence/commit-point-store.js").CommitPoint> {
  return desktopApi().createCommitPoint(streamId, batchId);
}

export async function deleteCommitPoint(id: string): Promise<void> {
  return desktopApi().deleteCommitPoint(id);
}

export async function updateCommitPoint(
  id: string,
  changes: { mode?: "auto" | "approve" },
): Promise<import("../persistence/commit-point-store.js").CommitPoint[]> {
  return desktopApi().updateCommitPoint(id, changes);
}

export async function commitCommitPoint(
  id: string,
  message: string,
): Promise<import("../persistence/commit-point-store.js").CommitPoint> {
  return desktopApi().commitCommitPoint(id, message);
}

export async function reorderBatchQueue(
  streamId: string,
  batchId: string,
  entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>,
): Promise<void> {
  return desktopApi().reorderBatchQueue(streamId, batchId, entries);
}

export async function listWaitPoints(batchId: string): Promise<import("../persistence/wait-point-store.js").WaitPoint[]> {
  return desktopApi().listWaitPoints(batchId);
}

export async function createWaitPoint(
  streamId: string,
  batchId: string,
  note?: string | null,
): Promise<import("../persistence/wait-point-store.js").WaitPoint> {
  return desktopApi().createWaitPoint(streamId, batchId, note);
}

export async function setWaitPointNote(
  id: string,
  note: string | null,
): Promise<import("../persistence/wait-point-store.js").WaitPoint> {
  return desktopApi().setWaitPointNote(id, note);
}

export async function deleteWaitPoint(id: string): Promise<void> {
  return desktopApi().deleteWaitPoint(id);
}

export async function listAllRefs(streamId: string): Promise<import("../git/git.js").RefOption[]> {
  return desktopApi().listAllRefs(streamId);
}

export async function addWorkItemNote(
  streamId: string,
  batchId: string,
  itemId: string,
  note: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().addWorkItemNote(streamId, batchId, itemId, note);
}

export async function listWorkItemEvents(
  streamId: string,
  batchId: string,
  itemId?: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().listWorkItemEvents(streamId, batchId, itemId);
}

export async function getWorkNotes(itemId: string): Promise<WorkNote[]> {
  return desktopApi().getWorkNotes(itemId);
}

export async function listAgentTurns(
  streamId: string,
  batchId: string,
  limit?: number,
): Promise<AgentTurn[]> {
  return desktopApi().listAgentTurns(streamId, batchId, limit);
}

export async function getBranchChanges(
  streamId: string,
  baseRef?: string,
): Promise<BranchChanges & { resolvedBaseRef: string | null }> {
  return desktopApi().getBranchChanges(streamId, baseRef);
}

export async function readFileAtRef(
  streamId: string,
  ref: string,
  path: string,
): Promise<{ content: string | null }> {
  return desktopApi().readFileAtRef(streamId, ref, path);
}

export async function listBatchFileChanges(
  streamId: string,
  batchId: string,
  limit?: number,
): Promise<BatchFileChange[]> {
  return desktopApi().listBatchFileChanges(streamId, batchId, limit);
}

export async function getTurnFileDiff(
  turnId: string,
  path: string,
): Promise<SnapshotDiffResult> {
  return desktopApi().getTurnFileDiff(turnId, path);
}

export async function listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]> {
  return desktopApi().listSnapshots(streamId, limit);
}

export async function getSnapshotSummary(snapshotId: string): Promise<SnapshotSummary | null> {
  return desktopApi().getSnapshotSummary(snapshotId);
}

export async function getSnapshotFileDiff(
  snapshotId: string,
  path: string,
): Promise<SnapshotDiffResult> {
  return desktopApi().getSnapshotFileDiff(snapshotId, path);
}

export async function getSnapshotPairDiff(
  beforeSnapshotId: string | null,
  afterSnapshotId: string,
  path: string,
): Promise<SnapshotDiffResult> {
  return desktopApi().getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path);
}

export async function restoreFileFromSnapshot(
  streamId: string,
  snapshotId: string,
  path: string,
): Promise<void> {
  return desktopApi().restoreFileFromSnapshot(streamId, snapshotId, path);
}

export interface FileSnapshotCreatedEventPayload {
  streamId: string;
  snapshotId: string;
  kind: SnapshotKind;
  turnId: string | null;
  batchId: string | null;
}

export function subscribeSnapshotEvents(
  streamId: string,
  fn: (payload: FileSnapshotCreatedEventPayload) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "file-snapshot.created") return;
    if (event.streamId !== streamId) return;
    fn({
      streamId: event.streamId,
      snapshotId: event.snapshotId,
      kind: event.kind,
      turnId: event.turnId,
      batchId: event.batchId,
    });
  });
}

export interface FileChangeRecordedEventPayload {
  streamId: string;
  batchId: string;
  turnId: string | null;
  changeId: string;
  path: string;
  kind: FileChangeKind;
  source: FileChangeSource;
}

export function subscribeFileChangeEvents(
  streamId: string | "all",
  onEvent: (event: FileChangeRecordedEventPayload) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "file-change.recorded") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({
      streamId: event.streamId,
      batchId: event.batchId,
      turnId: event.turnId,
      changeId: event.changeId,
      path: event.path,
      kind: event.kind,
      source: event.source,
    });
  });
}

export interface TurnChangeEvent {
  streamId: string;
  batchId: string;
  turnId: string;
  kind: "opened" | "closed";
}

export function subscribeTurnEvents(
  streamId: string | "all",
  onEvent: (event: TurnChangeEvent) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "turn.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({
      streamId: event.streamId,
      batchId: event.batchId,
      turnId: event.turnId,
      kind: event.kind,
    });
  });
}

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  return desktopApi().listWorkspaceEntries(streamId, path);
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: WorkspaceStatusSummary;
}> {
  return desktopApi().listWorkspaceFiles(streamId);
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return desktopApi().readWorkspaceFile(streamId, path);
}

export async function writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile> {
  return desktopApi().writeWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceFile(streamId: string, path: string, content = ""): Promise<WorkspaceFile> {
  return desktopApi().createWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().createWorkspaceDirectory(streamId, path);
}

export async function renameWorkspacePath(
  streamId: string,
  fromPath: string,
  toPath: string,
): Promise<WorkspaceRenameResult> {
  return desktopApi().renameWorkspacePath(streamId, fromPath, toPath);
}

export async function deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().deleteWorkspacePath(streamId, path);
}

export function subscribeNewdeEvents(
  listener: (event: NewdeEvent) => void,
): () => void {
  return desktopApi().onNewdeEvent(listener);
}

export function subscribeWorkspaceContext(
  onEvent: (next: WorkspaceContext) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "workspace-context.changed") return;
    onEvent({ gitEnabled: event.gitEnabled });
  });
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type === "workspace.changed" && event.streamId === streamId) {
      onEvent({
        id: event.id,
        streamId: event.streamId,
        kind: event.kind,
        path: event.path,
        t: event.t,
      });
    }
  });
}

export function subscribeGitRefsEvents(
  streamId: string,
  onEvent: () => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type === "git-refs.changed" && event.streamId === streamId) {
      onEvent();
    }
  });
}

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";

export interface WorkItemChangeEvent {
  streamId: string;
  batchId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export type AgentStatus = "idle" | "working" | "waiting" | "done";

export interface AgentStatusEntry {
  streamId: string;
  batchId: string;
  status: AgentStatus;
}

export async function listAgentStatuses(streamId?: string): Promise<AgentStatusEntry[]> {
  return desktopApi().listAgentStatuses(streamId);
}

export function subscribeAgentStatus(
  streamId: string | "all",
  onEvent: (entry: AgentStatusEntry) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "agent-status.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({ streamId: event.streamId, batchId: event.batchId, status: event.status });
  });
}

export interface BacklogChangeEvent {
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export function subscribeBacklogEvents(onEvent: (event: BacklogChangeEvent) => void): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "backlog.changed") return;
    onEvent({ kind: event.kind, itemId: event.itemId });
  });
}

export function subscribeWorkItemEvents(
  streamId: string | "all",
  onEvent: (event: WorkItemChangeEvent) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "work-item.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({
      streamId: event.streamId,
      batchId: event.batchId,
      kind: event.kind,
      itemId: event.itemId,
    });
  });
}

export async function probeDaemon(): Promise<boolean> {
  try {
    return await desktopApi().ping();
  } catch {
    return false;
  }
}

export type NormalizedEvent =
  | { kind: "session-start"; t: number; sessionId?: string; cwd?: string }
  | { kind: "session-end"; t: number; sessionId?: string; reason?: string }
  | { kind: "user-prompt"; t: number; sessionId?: string; prompt: string }
  | {
      kind: "tool-use-start";
      t: number;
      sessionId?: string;
      toolName: string;
      target?: string;
      input?: unknown;
    }
  | {
      kind: "tool-use-end";
      t: number;
      sessionId?: string;
      toolName: string;
      status: "ok" | "error";
    }
  | { kind: "stop"; t: number; sessionId?: string }
  | { kind: "notification"; t: number; sessionId?: string; message: string }
  | { kind: "meta"; t: number; sessionId?: string; hookEventName: string; raw: unknown };

export interface StoredEvent {
  id: number;
  streamId: string;
  batchId?: string;
  pane?: "working" | "talking";
  normalized: NormalizedEvent;
}

export async function listHookEvents(streamId?: string): Promise<StoredEvent[]> {
  return desktopApi().listHookEvents(streamId);
}

export function subscribeHookEvents(
  streamId: string | "all",
  onEvent: (event: StoredEvent) => void,
): () => void {
  return subscribeNewdeEvents((event) => {
    if (event.type !== "hook.recorded") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent(event.event as StoredEvent);
  });
}

function desktopApi(): DesktopApi {
  if (!window.newdeApi) {
    throw new Error("newde Electron API is unavailable");
  }
  return window.newdeApi;
}
