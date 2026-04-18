import { contextBridge, ipcRenderer } from "electron";
import type { CommandId, DesktopApi, LspEvent, NewdeEvent, TerminalEvent } from "./ipc-contract.js";

const api: DesktopApi = {
  getCurrentStream: () => ipcRenderer.invoke("newde:getCurrentStream"),
  listStreams: () => ipcRenderer.invoke("newde:listStreams"),
  switchStream: (id) => ipcRenderer.invoke("newde:switchStream", id),
  renameCurrentStream: (title) => ipcRenderer.invoke("newde:renameCurrentStream", title),
  listBranches: () => ipcRenderer.invoke("newde:listBranches"),
  getWorkspaceContext: () => ipcRenderer.invoke("newde:getWorkspaceContext"),
  createStream: (input) => ipcRenderer.invoke("newde:createStream", input),
  getBatchState: (streamId) => ipcRenderer.invoke("newde:getBatchState", streamId),
  createBatch: (streamId, title) => ipcRenderer.invoke("newde:createBatch", streamId, title),
  reorderBatch: (streamId, batchId, targetIndex) => ipcRenderer.invoke("newde:reorderBatch", streamId, batchId, targetIndex),
  selectBatch: (streamId, batchId) => ipcRenderer.invoke("newde:selectBatch", streamId, batchId),
  promoteBatch: (streamId, batchId) => ipcRenderer.invoke("newde:promoteBatch", streamId, batchId),
  completeBatch: (streamId, batchId) => ipcRenderer.invoke("newde:completeBatch", streamId, batchId),
  getBatchWorkState: (streamId, batchId) => ipcRenderer.invoke("newde:getBatchWorkState", streamId, batchId),
  createWorkItem: (streamId, batchId, input) => ipcRenderer.invoke("newde:createWorkItem", streamId, batchId, input),
  updateWorkItem: (streamId, batchId, itemId, changes) => ipcRenderer.invoke("newde:updateWorkItem", streamId, batchId, itemId, changes),
  deleteWorkItem: (streamId, batchId, itemId) => ipcRenderer.invoke("newde:deleteWorkItem", streamId, batchId, itemId),
  reorderWorkItems: (streamId, batchId, orderedItemIds) => ipcRenderer.invoke("newde:reorderWorkItems", streamId, batchId, orderedItemIds),
  moveWorkItemToBatch: (streamId, fromBatchId, itemId, toBatchId) => ipcRenderer.invoke("newde:moveWorkItemToBatch", streamId, fromBatchId, itemId, toBatchId),
  getBacklogState: () => ipcRenderer.invoke("newde:getBacklogState"),
  createBacklogItem: (input) => ipcRenderer.invoke("newde:createBacklogItem", input),
  updateBacklogItem: (itemId, changes) => ipcRenderer.invoke("newde:updateBacklogItem", itemId, changes),
  deleteBacklogItem: (itemId) => ipcRenderer.invoke("newde:deleteBacklogItem", itemId),
  reorderBacklog: (orderedItemIds) => ipcRenderer.invoke("newde:reorderBacklog", orderedItemIds),
  moveWorkItemToBacklog: (streamId, fromBatchId, itemId) => ipcRenderer.invoke("newde:moveWorkItemToBacklog", streamId, fromBatchId, itemId),
  moveBacklogItemToBatch: (streamId, itemId, toBatchId) => ipcRenderer.invoke("newde:moveBacklogItemToBatch", streamId, itemId, toBatchId),
  getGitLog: (streamId, options) => ipcRenderer.invoke("newde:getGitLog", streamId, options),
  getCommitDetail: (streamId, sha) => ipcRenderer.invoke("newde:getCommitDetail", streamId, sha),
  getChangeScopes: (streamId) => ipcRenderer.invoke("newde:getChangeScopes", streamId),
  searchWorkspaceText: (streamId, query, options) => ipcRenderer.invoke("newde:searchWorkspaceText", streamId, query, options),
  gitRestorePath: (streamId, path) => ipcRenderer.invoke("newde:gitRestorePath", streamId, path),
  gitAddPath: (streamId, path) => ipcRenderer.invoke("newde:gitAddPath", streamId, path),
  gitAppendToGitignore: (streamId, path) => ipcRenderer.invoke("newde:gitAppendToGitignore", streamId, path),
  gitPush: (streamId, options) => ipcRenderer.invoke("newde:gitPush", streamId, options),
  gitPull: (streamId, options) => ipcRenderer.invoke("newde:gitPull", streamId, options),
  listFileCommits: (streamId, path, limit) => ipcRenderer.invoke("newde:listFileCommits", streamId, path, limit),
  gitBlame: (streamId, path) => ipcRenderer.invoke("newde:gitBlame", streamId, path),
  listAllRefs: (streamId) => ipcRenderer.invoke("newde:listAllRefs", streamId),
  addWorkItemNote: (streamId, batchId, itemId, note) => ipcRenderer.invoke("newde:addWorkItemNote", streamId, batchId, itemId, note),
  listWorkItemEvents: (streamId, batchId, itemId) => ipcRenderer.invoke("newde:listWorkItemEvents", streamId, batchId, itemId),
  listAgentTurns: (streamId, batchId, limit) => ipcRenderer.invoke("newde:listAgentTurns", streamId, batchId, limit),
  listBatchFileChanges: (streamId, batchId, limit) => ipcRenderer.invoke("newde:listBatchFileChanges", streamId, batchId, limit),
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
  listCommitPoints: (batchId) => ipcRenderer.invoke("newde:listCommitPoints", batchId),
  createCommitPoint: (streamId, batchId, mode) => ipcRenderer.invoke("newde:createCommitPoint", streamId, batchId, mode),
  setCommitPointMode: (id, mode) => ipcRenderer.invoke("newde:setCommitPointMode", id, mode),
  approveCommitPoint: (id, editedMessage) => ipcRenderer.invoke("newde:approveCommitPoint", id, editedMessage),
  rejectCommitPoint: (id, note) => ipcRenderer.invoke("newde:rejectCommitPoint", id, note),
  resetCommitPoint: (id) => ipcRenderer.invoke("newde:resetCommitPoint", id),
  deleteCommitPoint: (id) => ipcRenderer.invoke("newde:deleteCommitPoint", id),
  listWaitPoints: (batchId) => ipcRenderer.invoke("newde:listWaitPoints", batchId),
  createWaitPoint: (streamId, batchId, note) => ipcRenderer.invoke("newde:createWaitPoint", streamId, batchId, note),
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
