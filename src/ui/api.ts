import type { DesktopApi, OxplowEvent } from "../electron/ipc-contract.js";

export type { OxplowEvent } from "../electron/ipc-contract.js";
export type { GitLogResult, GitLogCommit, GitLogRef, CommitDetail, ChangeScopes, TextSearchHit, GitOpResult, RefOption, BlameLine, GroupedGitRefs, GitWorktreeEntry } from "../git/git.js";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  branch_ref: string;
  branch_source: "local" | "remote" | "new";
  worktree_path: string;
  kind: "primary" | "worktree";
  created_at: string;
  updated_at: string;
  custom_prompt: string | null;
  panes: { working: string; talking: string };
  resume: { working_session_id: string; talking_session_id: string };
}

export interface Thread {
  id: string;
  stream_id: string;
  title: string;
  status: "active" | "queued" | "completed";
  sort_index: number;
  created_at: string;
  updated_at: string;
  pane_target: string;
  resume_session_id: string;
  custom_prompt: string | null;
}

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: Thread[];
}

export type WorkItemKind = "epic" | "task" | "subtask" | "bug" | "note";
export type WorkItemStatus = "ready" | "in_progress" | "human_check" | "blocked" | "done" | "canceled" | "archived";
export type WorkItemPriority = "low" | "medium" | "high" | "urgent";

export interface WorkItem {
  id: string;
  thread_id: string | null;
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
  /** Semantic origin of the row: `"agent"` for items the agent explicitly
   *  created via MCP, `"user"` for items the human filed, or `null` for
   *  legacy rows (including pre-v29 `agent-auto` rows that the store
   *  maps to null on read). See `WorkItemAuthor` in the persistence store. */
  author: "user" | "agent" | null;
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
  thread_id: string;
  item_id: string | null;
  event_type: string;
  actor_kind: "user" | "agent" | "system";
  actor_id: string;
  payload_json: string;
  created_at: string;
}

export type SnapshotSource =
  | "task-start"
  | "task-end"
  | "task-event"
  | "startup";

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  version_hash: string;
  source: SnapshotSource;
  created_at: string;
  label?: string | null;
  label_kind?: "task" | "turn" | "system" | null;
}

export type SnapshotEntryState = "present" | "oversize";

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
  previousSnapshotId: string | null;
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

export interface WorkItemEffort {
  id: string;
  work_item_id: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
  summary: string | null;
}

export interface EffortDetail {
  effort: WorkItemEffort;
  start_snapshot: FileSnapshot | null;
  end_snapshot: FileSnapshot | null;
  changed_paths: string[];
  counts: { created: number; updated: number; deleted: number };
}

export interface ThreadFollowup {
  id: string;
  note: string;
  createdAt: string;
}

export interface ThreadWorkState {
  threadId: string;
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
  epics: WorkItem[];
  items: WorkItem[];
  /** Transient agent follow-up reminders (in-memory on the runtime,
   *  lost on restart). Surfaced at the top of the To Do section. */
  followups: ThreadFollowup[];
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

export async function getConfig(): Promise<import("../config/config.js").OxplowConfig> {
  return desktopApi().getConfig();
}

export async function setAgentPromptAppend(text: string): Promise<import("../config/config.js").OxplowConfig> {
  return desktopApi().setAgentPromptAppend(text);
}

export async function setGeneratedDirs(dirs: string[]): Promise<import("../config/config.js").OxplowConfig> {
  return desktopApi().setGeneratedDirs(dirs);
}

export async function setSnapshotRetentionDays(days: number): Promise<import("../config/config.js").OxplowConfig> {
  return desktopApi().setSnapshotRetentionDays(days);
}

export async function setSnapshotMaxFileBytes(bytes: number): Promise<import("../config/config.js").OxplowConfig> {
  return desktopApi().setSnapshotMaxFileBytes(bytes);
}

export async function listBranches(): Promise<BranchRef[]> {
  return desktopApi().listBranches();
}

export async function listGitRefs(): Promise<import("../git/git.js").GroupedGitRefs> {
  return desktopApi().listGitRefs();
}

export async function renameGitBranch(from: string, to: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().renameGitBranch(from, to);
}

export async function deleteGitBranch(branch: string, options?: { force?: boolean }): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().deleteGitBranch(branch, options);
}

export async function gitMergeInto(streamId: string, other: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitMergeInto(streamId, other);
}

export async function gitRebaseOnto(streamId: string, onto: string): Promise<import("../git/git.js").GitOpResult> {
  return desktopApi().gitRebaseOnto(streamId, onto);
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  return desktopApi().getWorkspaceContext();
}

export async function createStream(input:
  | { title: string; summary?: string; source: "existing"; ref: string }
  | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string }
  | { title: string; summary?: string; source: "worktree"; worktreePath: string },
): Promise<Stream> {
  return desktopApi().createStream(input);
}

export async function listAdoptableWorktrees(): Promise<import("../git/git.js").GitWorktreeEntry[]> {
  return desktopApi().listAdoptableWorktrees();
}

export async function checkoutStreamBranch(streamId: string, branch: string): Promise<Stream> {
  return desktopApi().checkoutStreamBranch(streamId, branch);
}

export async function getThreadState(streamId: string): Promise<ThreadState> {
  return desktopApi().getThreadState(streamId);
}

export async function createThread(streamId: string, title: string): Promise<ThreadState> {
  return desktopApi().createThread(streamId, title);
}

export async function reorderThread(streamId: string, threadId: string, targetIndex: number): Promise<ThreadState> {
  return desktopApi().reorderThread(streamId, threadId, targetIndex);
}

export async function reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void> {
  return desktopApi().reorderThreads(streamId, orderedThreadIds);
}

export async function reorderStreams(orderedStreamIds: string[]): Promise<void> {
  return desktopApi().reorderStreams(orderedStreamIds);
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().selectThread(streamId, threadId);
}

export async function promoteThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().promoteThread(streamId, threadId);
}

export async function completeThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().completeThread(streamId, threadId);
}

export async function renameThread(streamId: string, threadId: string, title: string): Promise<Thread> {
  return desktopApi().renameThread(streamId, threadId, title);
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  return desktopApi().setStreamPrompt(streamId, prompt);
}

export async function setThreadPrompt(streamId: string, threadId: string, prompt: string | null): Promise<Thread[]> {
  return desktopApi().setThreadPrompt(streamId, threadId, prompt);
}

export async function getThreadWorkState(streamId: string, threadId: string): Promise<ThreadWorkState> {
  return desktopApi().getThreadWorkState(streamId, threadId);
}

export async function createWorkItem(
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
): Promise<ThreadWorkState> {
  return desktopApi().createWorkItem(streamId, threadId, input);
}

export async function updateWorkItem(
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
): Promise<ThreadWorkState> {
  return desktopApi().updateWorkItem(streamId, threadId, itemId, changes);
}

export async function deleteWorkItem(
  streamId: string,
  threadId: string,
  itemId: string,
): Promise<ThreadWorkState> {
  return desktopApi().deleteWorkItem(streamId, threadId, itemId);
}

export async function reorderWorkItems(
  streamId: string,
  threadId: string,
  orderedItemIds: string[],
): Promise<ThreadWorkState> {
  return desktopApi().reorderWorkItems(streamId, threadId, orderedItemIds);
}

export async function moveWorkItemToThread(
  streamId: string,
  fromThreadId: string,
  itemId: string,
  toThreadId: string,
  toStreamId?: string,
): Promise<{ from: ThreadWorkState; to: ThreadWorkState }> {
  return desktopApi().moveWorkItemToThread(streamId, fromThreadId, itemId, toThreadId, toStreamId);
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
  fromThreadId: string,
  itemId: string,
): Promise<{ from: ThreadWorkState; backlog: BacklogState }> {
  return desktopApi().moveWorkItemToBacklog(streamId, fromThreadId, itemId);
}

export async function moveBacklogItemToThread(
  streamId: string,
  itemId: string,
  toThreadId: string,
): Promise<{ backlog: BacklogState; to: ThreadWorkState }> {
  return desktopApi().moveBacklogItemToThread(streamId, itemId, toThreadId);
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

export type { LocalBlameEntry } from "../electron/local-blame.js";

export async function localBlame(
  streamId: string,
  path: string,
): Promise<import("../electron/local-blame.js").LocalBlameEntry[]> {
  return desktopApi().localBlame(streamId, path);
}

export type WikiNoteSummary = import("../electron/ipc-contract.js").WikiNoteSummary;
export type WikiNoteSearchHit = import("../electron/ipc-contract.js").WikiNoteSearchHit;
export type UsageRollup = import("../electron/ipc-contract.js").UsageRollup;

export async function listWikiNotes(streamId: string): Promise<WikiNoteSummary[]> {
  return desktopApi().listWikiNotes(streamId);
}

export async function readWikiNoteBody(streamId: string, slug: string): Promise<string> {
  return desktopApi().readWikiNoteBody(streamId, slug);
}

export async function writeWikiNoteBody(streamId: string, slug: string, body: string): Promise<void> {
  return desktopApi().writeWikiNoteBody(streamId, slug, body);
}

export async function deleteWikiNote(streamId: string, slug: string): Promise<void> {
  return desktopApi().deleteWikiNote(streamId, slug);
}

export function subscribeWikiNoteEvents(onEvent: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "wiki-note.changed") onEvent();
  });
}

export async function searchWikiNotes(
  streamId: string,
  query: string,
  limit?: number,
): Promise<WikiNoteSearchHit[]> {
  return desktopApi().searchWikiNotes(streamId, query, limit);
}

export async function recordUsage(input: {
  kind: string;
  key: string;
  event?: string;
  streamId?: string | null;
  threadId?: string | null;
}): Promise<void> {
  return desktopApi().recordUsage(input);
}

export async function listRecentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return desktopApi().listRecentUsage(input);
}

export async function listFrequentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return desktopApi().listFrequentUsage(input);
}

export async function listCurrentlyOpenUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
}): Promise<string[]> {
  return desktopApi().listCurrentlyOpenUsage(input);
}

export type CodeQualityTool = import("../electron/ipc-contract.js").CodeQualityTool;
export type CodeQualityScope = import("../electron/ipc-contract.js").CodeQualityScope;
export type CodeQualityScanStatus = import("../electron/ipc-contract.js").CodeQualityScanStatus;
export type CodeQualityFindingKind = import("../electron/ipc-contract.js").CodeQualityFindingKind;
export type CodeQualityScanRow = import("../electron/ipc-contract.js").CodeQualityScanRow;
export type CodeQualityFindingRow = import("../electron/ipc-contract.js").CodeQualityFindingRow;

export async function runCodeQualityScan(input: {
  streamId: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  baseRef?: string | null;
}): Promise<CodeQualityScanRow> {
  return desktopApi().runCodeQualityScan(input);
}

export async function listCodeQualityFindings(input: {
  streamId: string;
  tool?: CodeQualityTool;
  paths?: string[];
}): Promise<CodeQualityFindingRow[]> {
  return desktopApi().listCodeQualityFindings(input);
}

export async function listCodeQualityScans(input: {
  streamId: string;
  limit?: number;
}): Promise<CodeQualityScanRow[]> {
  return desktopApi().listCodeQualityScans(input);
}

export function subscribeCodeQualityEvents(
  streamId: string,
  fn: (event: { scanId: number; tool: CodeQualityTool; scope: CodeQualityScope; status: CodeQualityScanStatus }) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "code-quality.scanned") return;
    if (event.streamId !== streamId) return;
    fn({ scanId: event.scanId, tool: event.tool, scope: event.scope, status: event.status });
  });
}

export async function getWorkItemSummaries(ids: string[]): Promise<Array<{
  id: string;
  title: string;
  status: import("../persistence/work-item-store.js").WorkItemStatus;
  thread_id: string | null;
}>> {
  return desktopApi().getWorkItemSummaries(ids);
}

/**
 * Subscribe to `usage.recorded` events. Optionally filter by `kind` so a
 * Notes-pane consumer only refetches on wiki-note visits.
 */
export function subscribeUsageEvents(
  onEvent: (e: { kind: string; key: string; streamId: string | null; threadId: string | null }) => void,
  filter?: { kind?: string },
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "usage.recorded") return;
    if (filter?.kind && event.kind !== filter.kind) return;
    onEvent({ kind: event.kind, key: event.key, streamId: event.streamId, threadId: event.threadId });
  });
}

export async function reorderThreadQueue(
  streamId: string,
  threadId: string,
  entries: Array<{ id: string }>,
): Promise<void> {
  return desktopApi().reorderThreadQueue(streamId, threadId, entries);
}

export async function removeFollowup(threadId: string, id: string): Promise<void> {
  return desktopApi().removeFollowup(threadId, id);
}

export type BackgroundTask = import("../electron/background-task-store.js").BackgroundTask;

export async function listBackgroundTasks(): Promise<BackgroundTask[]> {
  return desktopApi().listBackgroundTasks();
}

export function subscribeBackgroundTaskEvents(
  onChange: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "background-task.changed") onChange();
  });
}

export async function listAllRefs(streamId: string): Promise<import("../git/git.js").RefOption[]> {
  return desktopApi().listAllRefs(streamId);
}

export async function addWorkItemNote(
  streamId: string,
  threadId: string,
  itemId: string,
  note: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().addWorkItemNote(streamId, threadId, itemId, note);
}

export async function listWorkItemEvents(
  streamId: string,
  threadId: string,
  itemId?: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().listWorkItemEvents(streamId, threadId, itemId);
}

export async function getWorkNotes(itemId: string): Promise<WorkNote[]> {
  return desktopApi().getWorkNotes(itemId);
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

export async function listWorkItemEfforts(itemId: string): Promise<EffortDetail[]> {
  return desktopApi().listWorkItemEfforts(itemId);
}

export async function listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]> {
  return desktopApi().listSnapshots(streamId, limit);
}

export async function getSnapshotSummary(
  snapshotId: string,
  previousSnapshotId?: string | null,
): Promise<SnapshotSummary | null> {
  return desktopApi().getSnapshotSummary(snapshotId, previousSnapshotId);
}

export async function getSnapshotPairDiff(
  beforeSnapshotId: string | null,
  afterSnapshotId: string,
  path: string,
): Promise<SnapshotDiffResult> {
  return desktopApi().getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path);
}

export async function getEffortFiles(effortId: string): Promise<SnapshotSummary | null> {
  return desktopApi().getEffortFiles(effortId);
}

export async function listEffortsEndingAtSnapshots(
  snapshotIds: string[],
): Promise<Record<string, Array<{ effortId: string; workItemId: string; threadId: string; title: string; status: WorkItemStatus; priority: WorkItemPriority }>>> {
  return desktopApi().listEffortsEndingAtSnapshots(snapshotIds);
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
  kind: SnapshotSource;
  effortId: string | null;
  threadId: string | null;
}

export function subscribeSnapshotEvents(
  streamId: string,
  fn: (payload: FileSnapshotCreatedEventPayload) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "file-snapshot.created") return;
    if (event.streamId !== streamId) return;
    fn({
      streamId: event.streamId,
      snapshotId: event.snapshotId,
      kind: event.kind,
      effortId: event.effortId,
      threadId: event.threadId,
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

export function subscribeOxplowEvents(
  listener: (event: OxplowEvent) => void,
): () => void {
  return desktopApi().onOxplowEvent(listener);
}

export function subscribeWorkspaceContext(
  onEvent: (next: WorkspaceContext) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "workspace-context.changed") return;
    onEvent({ gitEnabled: event.gitEnabled });
  });
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
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
  return subscribeOxplowEvents((event) => {
    if (event.type === "git-refs.changed" && event.streamId === streamId) {
      onEvent();
    }
  });
}

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";

export interface WorkItemChangeEvent {
  streamId: string;
  threadId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export type AgentStatus = "idle" | "working" | "waiting" | "done";

export interface AgentStatusEntry {
  streamId: string;
  threadId: string;
  status: AgentStatus;
}

export async function listAgentStatuses(streamId?: string): Promise<AgentStatusEntry[]> {
  return desktopApi().listAgentStatuses(streamId);
}

export function subscribeAgentStatus(
  streamId: string | "all",
  onEvent: (entry: AgentStatusEntry) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "agent-status.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({ streamId: event.streamId, threadId: event.threadId, status: event.status });
  });
}

export interface BacklogChangeEvent {
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export function subscribeBacklogEvents(onEvent: (event: BacklogChangeEvent) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "backlog.changed") return;
    onEvent({ kind: event.kind, itemId: event.itemId });
  });
}

export function subscribeWorkItemEvents(
  streamId: string | "all",
  onEvent: (event: WorkItemChangeEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "work-item.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({
      streamId: event.streamId,
      threadId: event.threadId,
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
  threadId?: string;
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
  return subscribeOxplowEvents((event) => {
    if (event.type !== "hook.recorded") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent(event.event as StoredEvent);
  });
}

function desktopApi(): DesktopApi {
  if (!window.oxplowApi) {
    throw new Error("oxplow Electron API is unavailable");
  }
  return window.oxplowApi;
}
