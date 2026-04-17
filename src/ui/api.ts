import type { DesktopApi, NewdeEvent } from "../electron/ipc-contract.js";

export type { NewdeEvent } from "../electron/ipc-contract.js";

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
  summary: string;
  summary_updated_at: string | null;
}

export interface BatchState {
  selectedBatchId: string | null;
  activeBatchId: string | null;
  batches: Batch[];
}

export type WorkItemKind = "epic" | "task" | "subtask" | "bug" | "note";
export type WorkItemStatus = "waiting" | "ready" | "in_progress" | "blocked" | "done" | "canceled";
export type WorkItemPriority = "low" | "medium" | "high" | "urgent";

export interface WorkItem {
  id: string;
  batch_id: string;
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

export interface AgentTurn {
  id: string;
  batch_id: string;
  work_item_id: string | null;
  prompt: string;
  answer: string | null;
  session_id: string | null;
  started_at: string;
  ended_at: string | null;
}

export interface BatchWorkState {
  batchId: string;
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
  epics: WorkItem[];
  items: WorkItem[];
}

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

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

export async function selectBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().selectBatch(streamId, batchId);
}

export async function promoteBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().promoteBatch(streamId, batchId);
}

export async function completeBatch(streamId: string, batchId: string): Promise<BatchState> {
  return desktopApi().completeBatch(streamId, batchId);
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

export async function listAgentTurns(
  streamId: string,
  batchId: string,
  limit?: number,
): Promise<AgentTurn[]> {
  return desktopApi().listAgentTurns(streamId, batchId, limit);
}

export async function listBatchFileChanges(
  streamId: string,
  batchId: string,
  limit?: number,
): Promise<BatchFileChange[]> {
  return desktopApi().listBatchFileChanges(streamId, batchId, limit);
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

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered";

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
