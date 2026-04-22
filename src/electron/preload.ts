import { contextBridge, ipcRenderer } from "electron";
import type { CommandId, DesktopApi, LspEvent, NewdeEvent, TerminalEvent } from "./ipc-contract.js";

// Each newde:event subscriber from the renderer adds one listener to
// ipcRenderer. The renderer has ~11 stores subscribing on startup
// (threads, streams, work items, backlog, agent status, turns, file
// changes, workspace events, workspace-context, etc.). Electron's
// default MaxListeners=10 fires a noisy MaxListenersExceededWarning on
// every launch. Raise the cap — these are long-lived per-store
// subscribers, not a leak. If this ever balloons further, switch to a
// single preload fan-out bus.
ipcRenderer.setMaxListeners(64);

const api: DesktopApi = {
  getCurrentStream: () => ipcRenderer.invoke("newde:getCurrentStream"),
  listStreams: () => ipcRenderer.invoke("newde:listStreams"),
  switchStream: (id) => ipcRenderer.invoke("newde:switchStream", id),
  renameCurrentStream: (title) => ipcRenderer.invoke("newde:renameCurrentStream", title),
  renameStream: (streamId, title) => ipcRenderer.invoke("newde:renameStream", streamId, title),
  getConfig: () => ipcRenderer.invoke("newde:getConfig"),
  setAgentPromptAppend: (text) => ipcRenderer.invoke("newde:setAgentPromptAppend", text),
  setSnapshotRetentionDays: (days) => ipcRenderer.invoke("newde:setSnapshotRetentionDays", days),
  setSnapshotMaxFileBytes: (bytes) => ipcRenderer.invoke("newde:setSnapshotMaxFileBytes", bytes),
  setGeneratedDirs: (dirs) => ipcRenderer.invoke("newde:setGeneratedDirs", dirs),
  listBranches: () => ipcRenderer.invoke("newde:listBranches"),
  getWorkspaceContext: () => ipcRenderer.invoke("newde:getWorkspaceContext"),
  createStream: (input) => ipcRenderer.invoke("newde:createStream", input),
  getThreadState: (streamId) => ipcRenderer.invoke("newde:getThreadState", streamId),
  createThread: (streamId, title) => ipcRenderer.invoke("newde:createThread", streamId, title),
  reorderThread: (streamId, threadId, targetIndex) => ipcRenderer.invoke("newde:reorderThread", streamId, threadId, targetIndex),
  reorderThreads: (streamId, orderedThreadIds) => ipcRenderer.invoke("newde:reorderThreads", streamId, orderedThreadIds),
  reorderStreams: (orderedStreamIds) => ipcRenderer.invoke("newde:reorderStreams", orderedStreamIds),
  selectThread: (streamId, threadId) => ipcRenderer.invoke("newde:selectThread", streamId, threadId),
  promoteThread: (streamId, threadId) => ipcRenderer.invoke("newde:promoteThread", streamId, threadId),
  completeThread: (streamId, threadId) => ipcRenderer.invoke("newde:completeThread", streamId, threadId),
  renameThread: (streamId, threadId, title) => ipcRenderer.invoke("newde:renameThread", streamId, threadId, title),
  setAutoCommit: (streamId, threadId, enabled) => ipcRenderer.invoke("newde:setAutoCommit", streamId, threadId, enabled),
  setStreamPrompt: (streamId, prompt) => ipcRenderer.invoke("newde:setStreamPrompt", streamId, prompt),
  setThreadPrompt: (streamId, threadId, prompt) => ipcRenderer.invoke("newde:setThreadPrompt", streamId, threadId, prompt),
  getThreadWorkState: (streamId, threadId) => ipcRenderer.invoke("newde:getThreadWorkState", streamId, threadId),
  createWorkItem: (streamId, threadId, input) => ipcRenderer.invoke("newde:createWorkItem", streamId, threadId, input),
  updateWorkItem: (streamId, threadId, itemId, changes) => ipcRenderer.invoke("newde:updateWorkItem", streamId, threadId, itemId, changes),
  deleteWorkItem: (streamId, threadId, itemId) => ipcRenderer.invoke("newde:deleteWorkItem", streamId, threadId, itemId),
  reorderWorkItems: (streamId, threadId, orderedItemIds) => ipcRenderer.invoke("newde:reorderWorkItems", streamId, threadId, orderedItemIds),
  moveWorkItemToThread: (streamId, fromThreadId, itemId, toThreadId, toStreamId) => ipcRenderer.invoke("newde:moveWorkItemToThread", streamId, fromThreadId, itemId, toThreadId, toStreamId),
  getBacklogState: () => ipcRenderer.invoke("newde:getBacklogState"),
  createBacklogItem: (input) => ipcRenderer.invoke("newde:createBacklogItem", input),
  updateBacklogItem: (itemId, changes) => ipcRenderer.invoke("newde:updateBacklogItem", itemId, changes),
  deleteBacklogItem: (itemId) => ipcRenderer.invoke("newde:deleteBacklogItem", itemId),
  reorderBacklog: (orderedItemIds) => ipcRenderer.invoke("newde:reorderBacklog", orderedItemIds),
  moveWorkItemToBacklog: (streamId, fromThreadId, itemId) => ipcRenderer.invoke("newde:moveWorkItemToBacklog", streamId, fromThreadId, itemId),
  moveBacklogItemToThread: (streamId, itemId, toThreadId) => ipcRenderer.invoke("newde:moveBacklogItemToThread", streamId, itemId, toThreadId),
  getGitLog: (streamId, options) => ipcRenderer.invoke("newde:getGitLog", streamId, options),
  getCommitDetail: (streamId, sha) => ipcRenderer.invoke("newde:getCommitDetail", streamId, sha),
  getChangeScopes: (streamId) => ipcRenderer.invoke("newde:getChangeScopes", streamId),
  searchWorkspaceText: (streamId, query, options) => ipcRenderer.invoke("newde:searchWorkspaceText", streamId, query, options),
  gitRestorePath: (streamId, path) => ipcRenderer.invoke("newde:gitRestorePath", streamId, path),
  gitAddPath: (streamId, path) => ipcRenderer.invoke("newde:gitAddPath", streamId, path),
  gitAppendToGitignore: (streamId, path) => ipcRenderer.invoke("newde:gitAppendToGitignore", streamId, path),
  gitPush: (streamId, options) => ipcRenderer.invoke("newde:gitPush", streamId, options),
  gitPull: (streamId, options) => ipcRenderer.invoke("newde:gitPull", streamId, options),
  gitCommitAll: (streamId, message, options) => ipcRenderer.invoke("newde:gitCommitAll", streamId, message, options),
  listFileCommits: (streamId, path, limit) => ipcRenderer.invoke("newde:listFileCommits", streamId, path, limit),
  gitBlame: (streamId, path) => ipcRenderer.invoke("newde:gitBlame", streamId, path),
  localBlame: (streamId, path) => ipcRenderer.invoke("newde:localBlame", streamId, path),
  listAllRefs: (streamId) => ipcRenderer.invoke("newde:listAllRefs", streamId),
  addWorkItemNote: (streamId, threadId, itemId, note) => ipcRenderer.invoke("newde:addWorkItemNote", streamId, threadId, itemId, note),
  listWorkItemEvents: (streamId, threadId, itemId) => ipcRenderer.invoke("newde:listWorkItemEvents", streamId, threadId, itemId),
  getWorkNotes: (itemId) => ipcRenderer.invoke("newde:getWorkNotes", itemId),
  listAgentTurns: (streamId, threadId, limit) => ipcRenderer.invoke("newde:listAgentTurns", streamId, threadId, limit),
  listWorkItemEfforts: (itemId) => ipcRenderer.invoke("newde:listWorkItemEfforts", itemId),
  listSnapshots: (streamId, limit) => ipcRenderer.invoke("newde:listSnapshots", streamId, limit),
  getSnapshotSummary: (snapshotId, previousSnapshotId) => ipcRenderer.invoke("newde:getSnapshotSummary", snapshotId, previousSnapshotId),
  getSnapshotPairDiff: (beforeSnapshotId, afterSnapshotId, path) => ipcRenderer.invoke("newde:getSnapshotPairDiff", beforeSnapshotId, afterSnapshotId, path),
  getEffortFiles: (effortId) => ipcRenderer.invoke("newde:getEffortFiles", effortId),
  listEffortsEndingAtSnapshots: (snapshotIds) => ipcRenderer.invoke("newde:listEffortsEndingAtSnapshots", snapshotIds),
  restoreFileFromSnapshot: (streamId, snapshotId, path) => ipcRenderer.invoke("newde:restoreFileFromSnapshot", streamId, snapshotId, path),
  getBranchChanges: (streamId, baseRef) => ipcRenderer.invoke("newde:getBranchChanges", streamId, baseRef),
  readFileAtRef: (streamId, ref, path) => ipcRenderer.invoke("newde:readFileAtRef", streamId, ref, path),
  listWorkspaceEntries: (streamId, path = "") => ipcRenderer.invoke("newde:listWorkspaceEntries", streamId, path),
  listWorkspaceFiles: (streamId) => ipcRenderer.invoke("newde:listWorkspaceFiles", streamId),
  readWorkspaceFile: (streamId, path) => ipcRenderer.invoke("newde:readWorkspaceFile", streamId, path),
  writeWorkspaceFile: (streamId, path, content) => ipcRenderer.invoke("newde:writeWorkspaceFile", streamId, path, content),
  createWorkspaceFile: (streamId, path, content = "") => ipcRenderer.invoke("newde:createWorkspaceFile", streamId, path, content),
  createWorkspaceDirectory: (streamId, path) => ipcRenderer.invoke("newde:createWorkspaceDirectory", streamId, path),
  renameWorkspacePath: (streamId, fromPath, toPath) => ipcRenderer.invoke("newde:renameWorkspacePath", streamId, fromPath, toPath),
  deleteWorkspacePath: (streamId, path) => ipcRenderer.invoke("newde:deleteWorkspacePath", streamId, path),
  listCommitPoints: (threadId) => ipcRenderer.invoke("newde:listCommitPoints", threadId),
  createCommitPoint: (streamId, threadId) => ipcRenderer.invoke("newde:createCommitPoint", streamId, threadId),
  deleteCommitPoint: (id) => ipcRenderer.invoke("newde:deleteCommitPoint", id),
  updateCommitPoint: (id, changes) => ipcRenderer.invoke("newde:updateCommitPoint", id, changes),
  commitCommitPoint: (id, message) => ipcRenderer.invoke("newde:commitCommitPoint", id, message),
  reorderThreadQueue: (streamId, threadId, entries) => ipcRenderer.invoke("newde:reorderThreadQueue", streamId, threadId, entries),
  listWaitPoints: (threadId) => ipcRenderer.invoke("newde:listWaitPoints", threadId),
  createWaitPoint: (streamId, threadId, note) => ipcRenderer.invoke("newde:createWaitPoint", streamId, threadId, note),
  setWaitPointNote: (id, note) => ipcRenderer.invoke("newde:setWaitPointNote", id, note),
  deleteWaitPoint: (id) => ipcRenderer.invoke("newde:deleteWaitPoint", id),
  listHookEvents: (streamId) => ipcRenderer.invoke("newde:listHookEvents", streamId),
  listAgentStatuses: (streamId) => ipcRenderer.invoke("newde:listAgentStatuses", streamId),
  ping: () => ipcRenderer.invoke("newde:ping"),
  logUi: (payload) => ipcRenderer.invoke("newde:logUi", payload),
  updateEditorFocus: (payload) => ipcRenderer.invoke("newde:updateEditorFocus", payload),
  setNativeMenu: (groups) => ipcRenderer.invoke("newde:setNativeMenu", groups),
  openTerminalSession: (paneTarget, cols, rows, mode = "direct") => ipcRenderer.invoke("newde:openTerminalSession", paneTarget, cols, rows, mode),
  sendTerminalMessage: (sessionId, message) => ipcRenderer.invoke("newde:sendTerminalMessage", sessionId, message),
  closeTerminalSession: (sessionId) => ipcRenderer.invoke("newde:closeTerminalSession", sessionId),
  openLspClient: (streamId, languageId) => ipcRenderer.invoke("newde:openLspClient", streamId, languageId),
  sendLspMessage: (clientId, message) => ipcRenderer.invoke("newde:sendLspMessage", clientId, message),
  closeLspClient: (clientId) => ipcRenderer.invoke("newde:closeLspClient", clientId),
  onNewdeEvent: (listener) => subscribe("newde:event", listener),
  onTerminalEvent: (listener) => subscribe("newde:terminal-event", listener),
  onLspEvent: (listener) => subscribe("newde:lsp-event", listener),
  onMenuCommand: (listener) => subscribe("newde:menu-command", listener),
};

contextBridge.exposeInMainWorld("newdeApi", api);
contextBridge.exposeInMainWorld("newdeDesktop", {
  isElectron: true,
  platform: process.platform,
});

declare global {
  interface Window {
    newdeApi: DesktopApi;
    newdeDesktop?: {
      isElectron: boolean;
      platform: NodeJS.Platform;
    };
  }
}

function subscribe<T extends NewdeEvent | TerminalEvent | LspEvent | CommandId>(
  channel: "newde:event" | "newde:terminal-event" | "newde:lsp-event" | "newde:menu-command",
  listener: (payload: T) => void,
) {
  const wrapped = (_event: unknown, payload: T) => listener(payload);
  ipcRenderer.on(channel, wrapped);
  return () => {
    ipcRenderer.removeListener(channel, wrapped);
  };
}
