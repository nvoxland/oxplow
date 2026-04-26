import { contextBridge, ipcRenderer } from "electron";
import type { CommandId, DesktopApi, LspEvent, OxplowEvent, TerminalEvent } from "./ipc-contract.js";

// Each oxplow:event subscriber from the renderer adds one listener to
// ipcRenderer. The renderer has ~11 stores subscribing on startup
// (threads, streams, work items, backlog, agent status, turns, file
// changes, workspace events, workspace-context, etc.). Electron's
// default MaxListeners=10 fires a noisy MaxListenersExceededWarning on
// every launch. Raise the cap — these are long-lived per-store
// subscribers, not a leak. If this ever balloons further, switch to a
// single preload fan-out bus.
ipcRenderer.setMaxListeners(64);

const api: DesktopApi = {
  getCurrentStream: () => ipcRenderer.invoke("oxplow:getCurrentStream"),
  listStreams: () => ipcRenderer.invoke("oxplow:listStreams"),
  switchStream: (id) => ipcRenderer.invoke("oxplow:switchStream", id),
  renameCurrentStream: (title) => ipcRenderer.invoke("oxplow:renameCurrentStream", title),
  renameStream: (streamId, title) => ipcRenderer.invoke("oxplow:renameStream", streamId, title),
  getConfig: () => ipcRenderer.invoke("oxplow:getConfig"),
  setAgentPromptAppend: (text) => ipcRenderer.invoke("oxplow:setAgentPromptAppend", text),
  setSnapshotRetentionDays: (days) => ipcRenderer.invoke("oxplow:setSnapshotRetentionDays", days),
  setSnapshotMaxFileBytes: (bytes) => ipcRenderer.invoke("oxplow:setSnapshotMaxFileBytes", bytes),
  setGeneratedDirs: (dirs) => ipcRenderer.invoke("oxplow:setGeneratedDirs", dirs),
  listBranches: () => ipcRenderer.invoke("oxplow:listBranches"),
  clipboardReadText: () => ipcRenderer.invoke("oxplow:clipboardReadText"),
  listGitRefs: () => ipcRenderer.invoke("oxplow:listGitRefs"),
  renameGitBranch: (from, to) => ipcRenderer.invoke("oxplow:renameGitBranch", from, to),
  deleteGitBranch: (branch, options) => ipcRenderer.invoke("oxplow:deleteGitBranch", branch, options),
  gitMergeInto: (streamId, other) => ipcRenderer.invoke("oxplow:gitMergeInto", streamId, other),
  gitRebaseOnto: (streamId, onto) => ipcRenderer.invoke("oxplow:gitRebaseOnto", streamId, onto),
  getWorkspaceContext: () => ipcRenderer.invoke("oxplow:getWorkspaceContext"),
  createStream: (input) => ipcRenderer.invoke("oxplow:createStream", input),
  listAdoptableWorktrees: () => ipcRenderer.invoke("oxplow:listAdoptableWorktrees"),
  checkoutStreamBranch: (streamId, branch) => ipcRenderer.invoke("oxplow:checkoutStreamBranch", streamId, branch),
  getThreadState: (streamId) => ipcRenderer.invoke("oxplow:getThreadState", streamId),
  createThread: (streamId, title) => ipcRenderer.invoke("oxplow:createThread", streamId, title),
  reorderThread: (streamId, threadId, targetIndex) => ipcRenderer.invoke("oxplow:reorderThread", streamId, threadId, targetIndex),
  reorderThreads: (streamId, orderedThreadIds) => ipcRenderer.invoke("oxplow:reorderThreads", streamId, orderedThreadIds),
  reorderStreams: (orderedStreamIds) => ipcRenderer.invoke("oxplow:reorderStreams", orderedStreamIds),
  selectThread: (streamId, threadId) => ipcRenderer.invoke("oxplow:selectThread", streamId, threadId),
  promoteThread: (streamId, threadId) => ipcRenderer.invoke("oxplow:promoteThread", streamId, threadId),
  completeThread: (streamId, threadId) => ipcRenderer.invoke("oxplow:completeThread", streamId, threadId),
  renameThread: (streamId, threadId, title) => ipcRenderer.invoke("oxplow:renameThread", streamId, threadId, title),
  setAutoCommit: (streamId, threadId, enabled) => ipcRenderer.invoke("oxplow:setAutoCommit", streamId, threadId, enabled),
  setStreamPrompt: (streamId, prompt) => ipcRenderer.invoke("oxplow:setStreamPrompt", streamId, prompt),
  setThreadPrompt: (streamId, threadId, prompt) => ipcRenderer.invoke("oxplow:setThreadPrompt", streamId, threadId, prompt),
  getThreadWorkState: (streamId, threadId) => ipcRenderer.invoke("oxplow:getThreadWorkState", streamId, threadId),
  createWorkItem: (streamId, threadId, input) => ipcRenderer.invoke("oxplow:createWorkItem", streamId, threadId, input),
  updateWorkItem: (streamId, threadId, itemId, changes) => ipcRenderer.invoke("oxplow:updateWorkItem", streamId, threadId, itemId, changes),
  deleteWorkItem: (streamId, threadId, itemId) => ipcRenderer.invoke("oxplow:deleteWorkItem", streamId, threadId, itemId),
  reorderWorkItems: (streamId, threadId, orderedItemIds) => ipcRenderer.invoke("oxplow:reorderWorkItems", streamId, threadId, orderedItemIds),
  moveWorkItemToThread: (streamId, fromThreadId, itemId, toThreadId, toStreamId) => ipcRenderer.invoke("oxplow:moveWorkItemToThread", streamId, fromThreadId, itemId, toThreadId, toStreamId),
  getBacklogState: () => ipcRenderer.invoke("oxplow:getBacklogState"),
  createBacklogItem: (input) => ipcRenderer.invoke("oxplow:createBacklogItem", input),
  updateBacklogItem: (itemId, changes) => ipcRenderer.invoke("oxplow:updateBacklogItem", itemId, changes),
  deleteBacklogItem: (itemId) => ipcRenderer.invoke("oxplow:deleteBacklogItem", itemId),
  reorderBacklog: (orderedItemIds) => ipcRenderer.invoke("oxplow:reorderBacklog", orderedItemIds),
  moveWorkItemToBacklog: (streamId, fromThreadId, itemId) => ipcRenderer.invoke("oxplow:moveWorkItemToBacklog", streamId, fromThreadId, itemId),
  moveBacklogItemToThread: (streamId, itemId, toThreadId) => ipcRenderer.invoke("oxplow:moveBacklogItemToThread", streamId, itemId, toThreadId),
  getGitLog: (streamId, options) => ipcRenderer.invoke("oxplow:getGitLog", streamId, options),
  getCommitDetail: (streamId, sha) => ipcRenderer.invoke("oxplow:getCommitDetail", streamId, sha),
  getChangeScopes: (streamId) => ipcRenderer.invoke("oxplow:getChangeScopes", streamId),
  searchWorkspaceText: (streamId, query, options) => ipcRenderer.invoke("oxplow:searchWorkspaceText", streamId, query, options),
  gitRestorePath: (streamId, path) => ipcRenderer.invoke("oxplow:gitRestorePath", streamId, path),
  gitAddPath: (streamId, path) => ipcRenderer.invoke("oxplow:gitAddPath", streamId, path),
  gitAppendToGitignore: (streamId, path) => ipcRenderer.invoke("oxplow:gitAppendToGitignore", streamId, path),
  gitPush: (streamId, options) => ipcRenderer.invoke("oxplow:gitPush", streamId, options),
  gitPull: (streamId, options) => ipcRenderer.invoke("oxplow:gitPull", streamId, options),
  gitCommitAll: (streamId, message, options) => ipcRenderer.invoke("oxplow:gitCommitAll", streamId, message, options),
  listFileCommits: (streamId, path, limit) => ipcRenderer.invoke("oxplow:listFileCommits", streamId, path, limit),
  gitBlame: (streamId, path) => ipcRenderer.invoke("oxplow:gitBlame", streamId, path),
  localBlame: (streamId, path) => ipcRenderer.invoke("oxplow:localBlame", streamId, path),
  listAllRefs: (streamId) => ipcRenderer.invoke("oxplow:listAllRefs", streamId),
  addWorkItemNote: (streamId, threadId, itemId, note) => ipcRenderer.invoke("oxplow:addWorkItemNote", streamId, threadId, itemId, note),
  listWorkItemEvents: (streamId, threadId, itemId) => ipcRenderer.invoke("oxplow:listWorkItemEvents", streamId, threadId, itemId),
  getWorkNotes: (itemId) => ipcRenderer.invoke("oxplow:getWorkNotes", itemId),
  listWorkItemEfforts: (itemId) => ipcRenderer.invoke("oxplow:listWorkItemEfforts", itemId),
  listSnapshots: (streamId, limit) => ipcRenderer.invoke("oxplow:listSnapshots", streamId, limit),
  getSnapshotSummary: (snapshotId, previousSnapshotId) => ipcRenderer.invoke("oxplow:getSnapshotSummary", snapshotId, previousSnapshotId),
  getSnapshotPairDiff: (beforeSnapshotId, afterSnapshotId, path) => ipcRenderer.invoke("oxplow:getSnapshotPairDiff", beforeSnapshotId, afterSnapshotId, path),
  getEffortFiles: (effortId) => ipcRenderer.invoke("oxplow:getEffortFiles", effortId),
  listEffortsEndingAtSnapshots: (snapshotIds) => ipcRenderer.invoke("oxplow:listEffortsEndingAtSnapshots", snapshotIds),
  restoreFileFromSnapshot: (streamId, snapshotId, path) => ipcRenderer.invoke("oxplow:restoreFileFromSnapshot", streamId, snapshotId, path),
  getBranchChanges: (streamId, baseRef) => ipcRenderer.invoke("oxplow:getBranchChanges", streamId, baseRef),
  readFileAtRef: (streamId, ref, path) => ipcRenderer.invoke("oxplow:readFileAtRef", streamId, ref, path),
  listWorkspaceEntries: (streamId, path = "") => ipcRenderer.invoke("oxplow:listWorkspaceEntries", streamId, path),
  listWorkspaceFiles: (streamId) => ipcRenderer.invoke("oxplow:listWorkspaceFiles", streamId),
  readWorkspaceFile: (streamId, path) => ipcRenderer.invoke("oxplow:readWorkspaceFile", streamId, path),
  writeWorkspaceFile: (streamId, path, content) => ipcRenderer.invoke("oxplow:writeWorkspaceFile", streamId, path, content),
  createWorkspaceFile: (streamId, path, content = "") => ipcRenderer.invoke("oxplow:createWorkspaceFile", streamId, path, content),
  createWorkspaceDirectory: (streamId, path) => ipcRenderer.invoke("oxplow:createWorkspaceDirectory", streamId, path),
  renameWorkspacePath: (streamId, fromPath, toPath) => ipcRenderer.invoke("oxplow:renameWorkspacePath", streamId, fromPath, toPath),
  deleteWorkspacePath: (streamId, path) => ipcRenderer.invoke("oxplow:deleteWorkspacePath", streamId, path),
  listWikiNotes: (streamId) => ipcRenderer.invoke("oxplow:listWikiNotes", streamId),
  readWikiNoteBody: (streamId, slug) => ipcRenderer.invoke("oxplow:readWikiNoteBody", streamId, slug),
  writeWikiNoteBody: (streamId, slug, body) => ipcRenderer.invoke("oxplow:writeWikiNoteBody", streamId, slug, body),
  deleteWikiNote: (streamId, slug) => ipcRenderer.invoke("oxplow:deleteWikiNote", streamId, slug),
  searchWikiNotes: (streamId, query, limit) => ipcRenderer.invoke("oxplow:searchWikiNotes", streamId, query, limit),
  recordUsage: (input) => ipcRenderer.invoke("oxplow:recordUsage", input),
  listRecentUsage: (input) => ipcRenderer.invoke("oxplow:listRecentUsage", input),
  listFrequentUsage: (input) => ipcRenderer.invoke("oxplow:listFrequentUsage", input),
  listCurrentlyOpenUsage: (input) => ipcRenderer.invoke("oxplow:listCurrentlyOpenUsage", input),
  runCodeQualityScan: (input) => ipcRenderer.invoke("oxplow:runCodeQualityScan", input),
  listCodeQualityFindings: (input) => ipcRenderer.invoke("oxplow:listCodeQualityFindings", input),
  listCodeQualityScans: (input) => ipcRenderer.invoke("oxplow:listCodeQualityScans", input),
  getWorkItemSummaries: (ids) => ipcRenderer.invoke("oxplow:getWorkItemSummaries", ids),
  listCommitPoints: (threadId) => ipcRenderer.invoke("oxplow:listCommitPoints", threadId),
  createCommitPoint: (streamId, threadId) => ipcRenderer.invoke("oxplow:createCommitPoint", streamId, threadId),
  deleteCommitPoint: (id) => ipcRenderer.invoke("oxplow:deleteCommitPoint", id),
  updateCommitPoint: (id, changes) => ipcRenderer.invoke("oxplow:updateCommitPoint", id, changes),
  commitCommitPoint: (id, message) => ipcRenderer.invoke("oxplow:commitCommitPoint", id, message),
  reorderThreadQueue: (streamId, threadId, entries) => ipcRenderer.invoke("oxplow:reorderThreadQueue", streamId, threadId, entries),
  listWaitPoints: (threadId) => ipcRenderer.invoke("oxplow:listWaitPoints", threadId),
  removeFollowup: (threadId, id) => ipcRenderer.invoke("oxplow:removeFollowup", threadId, id),
  listBackgroundTasks: () => ipcRenderer.invoke("oxplow:listBackgroundTasks"),
  createWaitPoint: (streamId, threadId, note) => ipcRenderer.invoke("oxplow:createWaitPoint", streamId, threadId, note),
  setWaitPointNote: (id, note) => ipcRenderer.invoke("oxplow:setWaitPointNote", id, note),
  deleteWaitPoint: (id) => ipcRenderer.invoke("oxplow:deleteWaitPoint", id),
  listHookEvents: (streamId) => ipcRenderer.invoke("oxplow:listHookEvents", streamId),
  listAgentStatuses: (streamId) => ipcRenderer.invoke("oxplow:listAgentStatuses", streamId),
  ping: () => ipcRenderer.invoke("oxplow:ping"),
  logUi: (payload) => ipcRenderer.invoke("oxplow:logUi", payload),
  updateEditorFocus: (payload) => ipcRenderer.invoke("oxplow:updateEditorFocus", payload),
  setNativeMenu: (groups) => ipcRenderer.invoke("oxplow:setNativeMenu", groups),
  openTerminalSession: (paneTarget, cols, rows, mode = "direct") => ipcRenderer.invoke("oxplow:openTerminalSession", paneTarget, cols, rows, mode),
  sendTerminalMessage: (sessionId, message) => ipcRenderer.invoke("oxplow:sendTerminalMessage", sessionId, message),
  closeTerminalSession: (sessionId) => ipcRenderer.invoke("oxplow:closeTerminalSession", sessionId),
  openLspClient: (streamId, languageId) => ipcRenderer.invoke("oxplow:openLspClient", streamId, languageId),
  sendLspMessage: (clientId, message) => ipcRenderer.invoke("oxplow:sendLspMessage", clientId, message),
  closeLspClient: (clientId) => ipcRenderer.invoke("oxplow:closeLspClient", clientId),
  onOxplowEvent: (listener) => subscribe("oxplow:event", listener),
  onTerminalEvent: (listener) => subscribe("oxplow:terminal-event", listener),
  onLspEvent: (listener) => subscribe("oxplow:lsp-event", listener),
  onMenuCommand: (listener) => subscribe("oxplow:menu-command", listener),
};

contextBridge.exposeInMainWorld("oxplowApi", api);
contextBridge.exposeInMainWorld("oxplowDesktop", {
  isElectron: true,
  platform: process.platform,
});

declare global {
  interface Window {
    oxplowApi: DesktopApi;
    oxplowDesktop?: {
      isElectron: boolean;
      platform: NodeJS.Platform;
    };
  }
}

function subscribe<T extends OxplowEvent | TerminalEvent | LspEvent | CommandId>(
  channel: "oxplow:event" | "oxplow:terminal-event" | "oxplow:lsp-event" | "oxplow:menu-command",
  listener: (payload: T) => void,
) {
  const wrapped = (_event: unknown, payload: T) => listener(payload);
  ipcRenderer.on(channel, wrapped);
  return () => {
    ipcRenderer.removeListener(channel, wrapped);
  };
}
