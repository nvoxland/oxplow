import { app, BrowserWindow, dialog, ipcMain, Menu, type MenuItemConstructorOptions } from "electron";
import { existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { ElectronRuntime } from "./runtime.js";
import type { CommandId, EditorFocusPayload, LspEvent, MenuGroupSnapshot, NewdeEvent, TerminalEvent, UiLogPayload } from "./ipc-contract.js";

let runtime: ElectronRuntime | null = null;
let mainWindow: BrowserWindow | null = null;
let quitting = false;
let disposed = false;
let instanceLockPath: string | null = null;

void main();

async function main() {
  const args = parseArgs(process.argv.slice(1));
  const projectDir = resolve(args.project ?? process.cwd());
  const openDevTools = args.devtools === "true";

  app.on("window-all-closed", () => {
    app.quit();
  });

  app.on("before-quit", (event) => {
    if (disposed || quitting) {
      return;
    }
    event.preventDefault();
    quitting = true;
    void disposeRuntime().finally(() => {
      releaseProjectLock();
      disposed = true;
      app.exit(0);
    });
  });

  app.on("activate", () => {
    if (!mainWindow && runtime) {
      mainWindow = createWindow(openDevTools, `NewDE: ${runtime.config.projectName}`);
    }
  });

  await app.whenReady();

  const lockResult = acquireProjectLock(projectDir);
  if (!lockResult.ok) {
    dialog.showErrorBox(
      "newde is already running for this project",
      `Another newde process (pid ${lockResult.pid}) is managing\n${projectDir}\n\nClose it (or kill that process) before starting a new one.`,
    );
    app.exit(1);
    return;
  }
  instanceLockPath = lockResult.lockPath;

  try {
    runtime = await ElectronRuntime.create(projectDir);
  } catch (error) {
    dialog.showErrorBox(
      "newde failed to start",
      error instanceof Error ? error.message : String(error),
    );
    releaseProjectLock();
    app.exit(1);
    return;
  }

  registerIpc(runtime);
  runtime.onEvent((event) => broadcast("newde:event", event));

  mainWindow = createWindow(openDevTools, `NewDE: ${runtime.config.projectName}`);
}

function createWindow(openDevTools: boolean, title: string) {
  const window = new BrowserWindow({
    width: 1440,
    height: 960,
    minWidth: 1000,
    minHeight: 700,
    backgroundColor: "#0e0e0e",
    title,
    webPreferences: {
      preload: resolve(app.getAppPath(), "dist", "electron-preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  window.on("closed", () => {
    if (mainWindow === window) {
      mainWindow = null;
    }
  });

  // Electron otherwise lets the page's <title> tag overwrite the window title
  // every time it fires `page-title-updated`, clobbering the per-project title
  // we set on the BrowserWindow.
  window.on("page-title-updated", (event) => {
    event.preventDefault();
  });

  void window.loadFile(resolve(app.getAppPath(), "public", "index.html"));
  if (openDevTools) {
    window.webContents.openDevTools({ mode: "detach" });
  }
  return window;
}

function registerIpc(currentRuntime: ElectronRuntime) {
  ipcMain.handle("newde:getCurrentStream", () => currentRuntime.getCurrentStream());
  ipcMain.handle("newde:listStreams", () => currentRuntime.listStreams());
  ipcMain.handle("newde:switchStream", (_event, id: string) => currentRuntime.switchStream(id));
  ipcMain.handle("newde:renameCurrentStream", (_event, title: string) => currentRuntime.renameCurrentStream(title));
  ipcMain.handle("newde:renameStream", (_event, streamId: string, title: string) => currentRuntime.renameStream(streamId, title));
  ipcMain.handle("newde:getConfig", () => currentRuntime.getConfig());
  ipcMain.handle("newde:setAgentPromptAppend", (_event, text: string) => currentRuntime.setAgentPromptAppend(text));
  ipcMain.handle("newde:setSnapshotRetentionDays", (_event, days: number) => currentRuntime.setSnapshotRetentionDays(days));
  ipcMain.handle("newde:setSnapshotMaxFileBytes", (_event, bytes: number) => currentRuntime.setSnapshotMaxFileBytes(bytes));
  ipcMain.handle("newde:setGeneratedDirs", (_event, dirs: string[]) => currentRuntime.setGeneratedDirs(dirs));
  ipcMain.handle("newde:listBranches", () => currentRuntime.listBranches());
  ipcMain.handle("newde:getWorkspaceContext", () => currentRuntime.getWorkspaceContext());
  ipcMain.handle("newde:createStream", (_event, input) => currentRuntime.createStream(input));
  ipcMain.handle("newde:getBatchState", (_event, streamId: string) => currentRuntime.getBatchState(streamId));
  ipcMain.handle("newde:createBatch", (_event, streamId: string, title: string) => currentRuntime.createBatch(streamId, title));
  ipcMain.handle("newde:reorderBatch", (_event, streamId: string, batchId: string, targetIndex: number) => currentRuntime.reorderBatch(streamId, batchId, targetIndex));
  ipcMain.handle("newde:selectBatch", (_event, streamId: string, batchId: string) => currentRuntime.selectBatch(streamId, batchId));
  ipcMain.handle("newde:promoteBatch", (_event, streamId: string, batchId: string) => currentRuntime.promoteBatch(streamId, batchId));
  ipcMain.handle("newde:completeBatch", (_event, streamId: string, batchId: string) => currentRuntime.completeBatch(streamId, batchId));
  ipcMain.handle("newde:renameBatch", (_event, streamId: string, batchId: string, title: string) => currentRuntime.renameBatch(streamId, batchId, title));
  ipcMain.handle("newde:getBatchWorkState", (_event, streamId: string, batchId: string) => currentRuntime.workItemApi.getBatchWorkState(streamId, batchId));
  ipcMain.handle("newde:createWorkItem", (_event, streamId: string, batchId: string, input) => currentRuntime.workItemApi.createWorkItem(streamId, batchId, input));
  ipcMain.handle("newde:updateWorkItem", (_event, streamId: string, batchId: string, itemId: string, changes) => currentRuntime.workItemApi.updateWorkItem(streamId, batchId, itemId, changes));
  ipcMain.handle("newde:deleteWorkItem", (_event, streamId: string, batchId: string, itemId: string) => currentRuntime.workItemApi.deleteWorkItem(streamId, batchId, itemId));
  ipcMain.handle("newde:reorderWorkItems", (_event, streamId: string, batchId: string, orderedItemIds: string[]) => currentRuntime.workItemApi.reorderWorkItems(streamId, batchId, orderedItemIds));
  ipcMain.handle("newde:moveWorkItemToBatch", (_event, streamId: string, fromBatchId: string, itemId: string, toBatchId: string) => currentRuntime.workItemApi.moveWorkItemToBatch(streamId, fromBatchId, itemId, toBatchId));
  ipcMain.handle("newde:getBacklogState", () => currentRuntime.workItemApi.getBacklogState());
  ipcMain.handle("newde:createBacklogItem", (_event, input) => currentRuntime.workItemApi.createBacklogItem(input));
  ipcMain.handle("newde:updateBacklogItem", (_event, itemId: string, changes) => currentRuntime.workItemApi.updateBacklogItem(itemId, changes));
  ipcMain.handle("newde:deleteBacklogItem", (_event, itemId: string) => currentRuntime.workItemApi.deleteBacklogItem(itemId));
  ipcMain.handle("newde:reorderBacklog", (_event, orderedItemIds: string[]) => currentRuntime.workItemApi.reorderBacklog(orderedItemIds));
  ipcMain.handle("newde:moveWorkItemToBacklog", (_event, streamId: string, fromBatchId: string, itemId: string) => currentRuntime.workItemApi.moveWorkItemToBacklog(streamId, fromBatchId, itemId));
  ipcMain.handle("newde:moveBacklogItemToBatch", (_event, streamId: string, itemId: string, toBatchId: string) => currentRuntime.workItemApi.moveBacklogItemToBatch(streamId, itemId, toBatchId));
  ipcMain.handle("newde:getGitLog", (_event, streamId: string, options?: { limit?: number }) => currentRuntime.getGitLog(streamId, options));
  ipcMain.handle("newde:getCommitDetail", (_event, streamId: string, sha: string) => currentRuntime.getCommitDetail(streamId, sha));
  ipcMain.handle("newde:getChangeScopes", (_event, streamId: string) => currentRuntime.getChangeScopes(streamId));
  ipcMain.handle("newde:searchWorkspaceText", (_event, streamId: string, query: string, options?: { limit?: number }) => currentRuntime.searchWorkspaceText(streamId, query, options));
  ipcMain.handle("newde:gitRestorePath", (_event, streamId: string, path: string) => currentRuntime.gitRestorePath(streamId, path));
  ipcMain.handle("newde:gitAddPath", (_event, streamId: string, path: string) => currentRuntime.gitAddPath(streamId, path));
  ipcMain.handle("newde:gitAppendToGitignore", (_event, streamId: string, path: string) => currentRuntime.gitAppendToGitignore(streamId, path));
  ipcMain.handle("newde:gitPush", (_event, streamId: string, options) => currentRuntime.gitPush(streamId, options));
  ipcMain.handle("newde:gitPull", (_event, streamId: string, options) => currentRuntime.gitPull(streamId, options));
  ipcMain.handle("newde:listFileCommits", (_event, streamId: string, path: string, limit?: number) => currentRuntime.listFileCommits(streamId, path, limit));
  ipcMain.handle("newde:gitBlame", (_event, streamId: string, path: string) => currentRuntime.gitBlame(streamId, path));
  ipcMain.handle("newde:listAllRefs", (_event, streamId: string) => currentRuntime.listAllRefs(streamId));
  ipcMain.handle("newde:addWorkItemNote", (_event, streamId: string, batchId: string, itemId: string, note: string) => currentRuntime.workItemApi.addWorkItemNote(streamId, batchId, itemId, note));
  ipcMain.handle("newde:listWorkItemEvents", (_event, streamId: string, batchId: string, itemId?: string) => currentRuntime.workItemApi.listWorkItemEvents(streamId, batchId, itemId));
  ipcMain.handle("newde:listAgentTurns", (_event, streamId: string, batchId: string, limit?: number) => currentRuntime.workItemApi.listAgentTurns(streamId, batchId, limit));
  ipcMain.handle("newde:listBatchFileChanges", (_event, streamId: string, batchId: string, limit?: number) => currentRuntime.workItemApi.listFileChanges(streamId, batchId, limit));
  ipcMain.handle("newde:getTurnFileDiff", (_event, turnId: string, path: string) => currentRuntime.getTurnFileDiff(turnId, path));
  ipcMain.handle("newde:listSnapshots", (_event, streamId: string, limit?: number) => currentRuntime.listSnapshots(streamId, limit));
  ipcMain.handle("newde:getSnapshotSummary", (_event, snapshotId: string) => currentRuntime.getSnapshotSummary(snapshotId));
  ipcMain.handle("newde:getSnapshotFileDiff", (_event, snapshotId: string, path: string) => currentRuntime.getSnapshotFileDiff(snapshotId, path));
  ipcMain.handle("newde:getSnapshotPairDiff", (_event, beforeSnapshotId: string | null, afterSnapshotId: string, path: string) => currentRuntime.getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path));
  ipcMain.handle("newde:restoreFileFromSnapshot", (_event, streamId: string, snapshotId: string, path: string) => currentRuntime.restoreFileFromSnapshot(streamId, snapshotId, path));
  ipcMain.handle("newde:getBranchChanges", (_event, streamId: string, baseRef?: string) => currentRuntime.getBranchChanges(streamId, baseRef));
  ipcMain.handle("newde:readFileAtRef", (_event, streamId: string, ref: string, path: string) => currentRuntime.readFileAtRef(streamId, ref, path));
  ipcMain.handle("newde:listWorkspaceEntries", (_event, streamId: string, path?: string) => currentRuntime.listWorkspaceEntries(streamId, path));
  ipcMain.handle("newde:listWorkspaceFiles", (_event, streamId: string) => currentRuntime.listWorkspaceFiles(streamId));
  ipcMain.handle("newde:readWorkspaceFile", (_event, streamId: string, path: string) => currentRuntime.readWorkspaceFile(streamId, path));
  ipcMain.handle("newde:writeWorkspaceFile", (_event, streamId: string, path: string, content: string) => currentRuntime.writeWorkspaceFile(streamId, path, content));
  ipcMain.handle("newde:createWorkspaceFile", (_event, streamId: string, path: string, content?: string) => currentRuntime.createWorkspaceFile(streamId, path, content));
  ipcMain.handle("newde:createWorkspaceDirectory", (_event, streamId: string, path: string) => currentRuntime.createWorkspaceDirectory(streamId, path));
  ipcMain.handle("newde:renameWorkspacePath", (_event, streamId: string, fromPath: string, toPath: string) => currentRuntime.renameWorkspacePath(streamId, fromPath, toPath));
  ipcMain.handle("newde:deleteWorkspacePath", (_event, streamId: string, path: string) => currentRuntime.deleteWorkspacePath(streamId, path));
  ipcMain.handle("newde:listCommitPoints", (_event, batchId: string) => currentRuntime.listCommitPoints(batchId));
  ipcMain.handle("newde:createCommitPoint", (_event, streamId: string, batchId: string, mode: "auto" | "approval") => currentRuntime.createCommitPoint(streamId, batchId, mode));
  ipcMain.handle("newde:setCommitPointMode", (_event, id: string, mode: "auto" | "approval") => currentRuntime.setCommitPointMode(id, mode));
  ipcMain.handle("newde:approveCommitPoint", (_event, id: string, editedMessage?: string) => currentRuntime.approveCommitPoint(id, editedMessage));
  ipcMain.handle("newde:rejectCommitPoint", (_event, id: string, note: string) => currentRuntime.rejectCommitPoint(id, note));
  ipcMain.handle("newde:resetCommitPoint", (_event, id: string) => currentRuntime.resetCommitPoint(id));
  ipcMain.handle("newde:deleteCommitPoint", (_event, id: string) => currentRuntime.deleteCommitPoint(id));
  ipcMain.handle("newde:reorderBatchQueue", (_event, streamId: string, batchId: string, entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>) => currentRuntime.reorderBatchQueue(streamId, batchId, entries));
  ipcMain.handle("newde:listWaitPoints", (_event, batchId: string) => currentRuntime.listWaitPoints(batchId));
  ipcMain.handle("newde:createWaitPoint", (_event, streamId: string, batchId: string, note?: string | null) => currentRuntime.createWaitPoint(streamId, batchId, note));
  ipcMain.handle("newde:setWaitPointNote", (_event, id: string, note: string | null) => currentRuntime.setWaitPointNote(id, note));
  ipcMain.handle("newde:deleteWaitPoint", (_event, id: string) => currentRuntime.deleteWaitPoint(id));
  ipcMain.handle("newde:listHookEvents", (_event, streamId?: string) => currentRuntime.listHookEvents(streamId));
  ipcMain.handle("newde:listAgentStatuses", (_event, streamId?: string) => currentRuntime.listAgentStatuses(streamId));
  ipcMain.handle("newde:ping", () => currentRuntime.ping());
  ipcMain.handle("newde:logUi", (_event, payload: UiLogPayload) => currentRuntime.logUi(payload));
  ipcMain.handle("newde:updateEditorFocus", (_event, payload: EditorFocusPayload) => currentRuntime.updateEditorFocus(payload));
  ipcMain.handle("newde:setNativeMenu", (_event, groups: MenuGroupSnapshot[]) => {
    Menu.setApplicationMenu(buildNativeMenu(groups));
  });
  ipcMain.handle("newde:openTerminalSession", (_event, paneTarget: string, cols: number, rows: number, mode: "direct" | "tmux" = "direct") =>
    currentRuntime.openTerminalSession(paneTarget, cols, rows, mode, (sessionId, message) => {
      const payload: TerminalEvent = { sessionId, message };
      broadcast("newde:terminal-event", payload);
    }),
  );
  ipcMain.handle("newde:sendTerminalMessage", (_event, sessionId: string, message: string) =>
    currentRuntime.sendTerminalMessage(sessionId, message),
  );
  ipcMain.handle("newde:closeTerminalSession", (_event, sessionId: string) =>
    currentRuntime.closeTerminalSession(sessionId),
  );
  ipcMain.handle("newde:openLspClient", (_event, streamId: string, languageId: string) =>
    currentRuntime.openLspClient(streamId, languageId, (clientId, message) => {
      const payload: LspEvent = { clientId, message };
      broadcast("newde:lsp-event", payload);
    }),
  );
  ipcMain.handle("newde:sendLspMessage", (_event, clientId: string, message: string) =>
    currentRuntime.sendLspMessage(clientId, message),
  );
  ipcMain.handle("newde:closeLspClient", (_event, clientId: string) =>
    currentRuntime.closeLspClient(clientId),
  );
}

function broadcast(
  channel: "newde:event" | "newde:terminal-event" | "newde:lsp-event",
  payload: NewdeEvent | TerminalEvent | LspEvent,
) {
  for (const window of BrowserWindow.getAllWindows()) {
    if (!window.isDestroyed()) {
      window.webContents.send(channel, payload);
    }
  }
}

function sendMenuCommand(commandId: CommandId) {
  const targetWindow = BrowserWindow.getFocusedWindow() ?? mainWindow;
  if (targetWindow && !targetWindow.isDestroyed()) {
    targetWindow.webContents.send("newde:menu-command", commandId);
  }
}

function buildNativeMenu(groups: MenuGroupSnapshot[]) {
  const template: MenuItemConstructorOptions[] = [];
  if (process.platform === "darwin") {
    template.push({
      label: app.name,
      submenu: [
        { role: "about" },
        { type: "separator" },
        { role: "services" },
        { type: "separator" },
        { role: "hide" },
        { role: "hideOthers" },
        { role: "unhide" },
        { type: "separator" },
        { role: "quit" },
      ],
    });
  }
  for (const group of groups) {
    template.push({
      label: group.label,
      submenu: buildNativeSubmenu(group),
    });
  }
  if (process.platform === "darwin") {
    template.push({
      label: "Window",
      role: "window",
      submenu: [{ role: "minimize" }, { role: "zoom" }, { type: "separator" }, { role: "front" }],
    });
  }
  return Menu.buildFromTemplate(template);
}

function buildNativeSubmenu(group: MenuGroupSnapshot): MenuItemConstructorOptions[] {
  const items = group.items.map((item): MenuItemConstructorOptions => ({
    label: item.label,
    type: typeof item.checked === "boolean" ? "checkbox" : "normal",
    checked: item.checked,
    enabled: item.enabled ?? true,
    accelerator: item.shortcut ? toElectronAccelerator(item.shortcut) : undefined,
    click: () => sendMenuCommand(item.id),
  }));
  if (group.id !== "edit") {
    return items;
  }
  return [
    { role: "undo" },
    { role: "redo" },
    { type: "separator" },
    { role: "cut" },
    { role: "copy" },
    { role: "paste" },
    { role: "selectAll" },
    { type: "separator" },
    ...items,
  ];
}

function toElectronAccelerator(shortcut: string) {
  return shortcut.replace("Ctrl/Cmd", "CommandOrControl");
}

async function disposeRuntime() {
  const currentRuntime = runtime;
  runtime = null;
  if (currentRuntime) {
    await currentRuntime.dispose();
  }
}

type LockResult =
  | { ok: true; lockPath: string }
  | { ok: false; pid: number; lockPath: string };

/**
 * Claims an exclusive per-project lock at `.newde/runtime/instance.lock`
 * containing this process's PID. Refuses to start if another live process
 * already holds the lock; reclaims stale locks whose PID no longer exists.
 * Per-project means two different projects can each have their own newde.
 */
function acquireProjectLock(projectDir: string): LockResult {
  const runtimeDir = join(projectDir, ".newde", "runtime");
  mkdirSync(runtimeDir, { recursive: true });
  const lockPath = join(runtimeDir, "instance.lock");
  if (existsSync(lockPath)) {
    const priorPid = readLockPid(lockPath);
    if (priorPid != null && priorPid !== process.pid && isPidAlive(priorPid)) {
      return { ok: false, pid: priorPid, lockPath };
    }
    // stale — either the writer died without cleaning up, or it's our own pid
    try { unlinkSync(lockPath); } catch (err) {
      console.warn("[newde] could not remove stale lock file", lockPath, err);
    }
  }
  writeFileSync(lockPath, String(process.pid), "utf8");
  return { ok: true, lockPath };
}

function releaseProjectLock(): void {
  if (!instanceLockPath) return;
  try {
    const currentPid = readLockPid(instanceLockPath);
    if (currentPid === process.pid) unlinkSync(instanceLockPath);
  } catch (err) {
    console.warn("[newde] could not release project lock", instanceLockPath, err);
  }
  instanceLockPath = null;
}

function readLockPid(lockPath: string): number | null {
  try {
    const raw = readFileSync(lockPath, "utf8").trim();
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

// `process.kill(pid, 0)` probes without delivering a signal; throws ESRCH if
// the process doesn't exist and EPERM if it does but we can't signal it.
function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "EPERM") return true;
    return false;
  }
}

function parseArgs(argv: string[]) {
  const args: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const candidate = argv[i];
    if (!candidate.startsWith("--")) continue;
    const key = candidate.slice(2);
    const next = argv[i + 1];
    if (next && !next.startsWith("--")) {
      args[key] = next;
      i++;
    } else {
      args[key] = "true";
    }
  }
  return args;
}
