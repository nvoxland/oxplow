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
  addWorkItemNote: (streamId, batchId, itemId, note) => ipcRenderer.invoke("newde:addWorkItemNote", streamId, batchId, itemId, note),
  listWorkItemEvents: (streamId, batchId, itemId) => ipcRenderer.invoke("newde:listWorkItemEvents", streamId, batchId, itemId),
  listWorkspaceEntries: (streamId, path = "") => ipcRenderer.invoke("newde:listWorkspaceEntries", streamId, path),
  listWorkspaceFiles: (streamId) => ipcRenderer.invoke("newde:listWorkspaceFiles", streamId),
  readWorkspaceFile: (streamId, path) => ipcRenderer.invoke("newde:readWorkspaceFile", streamId, path),
  writeWorkspaceFile: (streamId, path, content) => ipcRenderer.invoke("newde:writeWorkspaceFile", streamId, path, content),
  createWorkspaceFile: (streamId, path, content = "") => ipcRenderer.invoke("newde:createWorkspaceFile", streamId, path, content),
  createWorkspaceDirectory: (streamId, path) => ipcRenderer.invoke("newde:createWorkspaceDirectory", streamId, path),
  renameWorkspacePath: (streamId, fromPath, toPath) => ipcRenderer.invoke("newde:renameWorkspacePath", streamId, fromPath, toPath),
  deleteWorkspacePath: (streamId, path) => ipcRenderer.invoke("newde:deleteWorkspacePath", streamId, path),
  listHookEvents: (streamId) => ipcRenderer.invoke("newde:listHookEvents", streamId),
  listAgentStatuses: (streamId) => ipcRenderer.invoke("newde:listAgentStatuses", streamId),
  ping: () => ipcRenderer.invoke("newde:ping"),
  logUi: (payload) => ipcRenderer.invoke("newde:logUi", payload),
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
