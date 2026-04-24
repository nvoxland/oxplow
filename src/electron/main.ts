import { app, BrowserWindow, dialog, ipcMain, Menu, shell, type MenuItemConstructorOptions } from "electron";
import { existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { ElectronRuntime } from "./runtime.js";
import type { CommandId, EditorFocusPayload, LspEvent, MenuGroupSnapshot, OxplowEvent, TerminalEvent, UiLogPayload } from "./ipc-contract.js";

let runtime: ElectronRuntime | null = null;
let mainWindow: BrowserWindow | null = null;
let quitting = false;
let disposed = false;
let instanceLockPath: string | null = null;
const registeredIpcChannels: string[] = [];

// Wrapper around ipcMain.handle that records the channel so we can
// remove every handler before the runtime/SQLite database closes. Without
// this, late in-flight renderer requests (e.g. listWorkspaceFiles) hit
// the runtime after dispose() and surface as "Error: database is not
// open" in the logs during quit.
function handle(channel: string, listener: Parameters<typeof ipcMain.handle>[1]): void {
  ipcMain.handle(channel, listener);
  registeredIpcChannels.push(channel);
}

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
      mainWindow = createWindow(openDevTools, `Oxplow: ${runtime.config.projectName}`);
    }
  });

  await app.whenReady();

  const lockResult = acquireProjectLock(projectDir);
  if (!lockResult.ok) {
    dialog.showErrorBox(
      "oxplow is already running for this project",
      `Another oxplow process (pid ${lockResult.pid}) is managing\n${projectDir}\n\nClose it (or kill that process) before starting a new one.`,
    );
    app.exit(1);
    return;
  }
  instanceLockPath = lockResult.lockPath;

  try {
    runtime = await ElectronRuntime.create(projectDir);
  } catch (error) {
    dialog.showErrorBox(
      "oxplow failed to start",
      error instanceof Error ? error.message : String(error),
    );
    releaseProjectLock();
    app.exit(1);
    return;
  }

  registerIpc(runtime);
  runtime.onEvent((event) => broadcast("oxplow:event", event));

  mainWindow = createWindow(openDevTools, `Oxplow: ${runtime.config.projectName}`);
}

function getWindowBoundsPath(): string {
  return join(app.getPath("userData"), "window-bounds.json");
}

function loadSavedBounds(): { x: number; y: number; width: number; height: number } | null {
  try {
    const path = getWindowBoundsPath();
    if (!existsSync(path)) return null;
    const raw = readFileSync(path, "utf8");
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed.x === "number" &&
      typeof parsed.y === "number" &&
      typeof parsed.width === "number" &&
      typeof parsed.height === "number"
    ) {
      return { x: parsed.x, y: parsed.y, width: parsed.width, height: parsed.height };
    }
    return null;
  } catch (error) {
    console.warn("[oxplow] failed to load window bounds:", error);
    return null;
  }
}

function saveBounds(window: BrowserWindow): void {
  try {
    const dir = app.getPath("userData");
    if (!existsSync(dir)) {
      mkdirSync(dir, { recursive: true });
    }
    const bounds = window.getBounds();
    writeFileSync(getWindowBoundsPath(), JSON.stringify(bounds), "utf8");
  } catch (error) {
    console.warn("[oxplow] failed to save window bounds:", error);
  }
}

function createWindow(openDevTools: boolean, title: string) {
  const savedBounds = loadSavedBounds();
  const window = new BrowserWindow({
    ...(savedBounds ?? { width: 1440, height: 960 }),
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

  let saveTimer: NodeJS.Timeout | null = null;
  const scheduleSave = () => {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      saveBounds(window);
    }, 500);
  };

  window.on("move", scheduleSave);
  window.on("resize", scheduleSave);
  window.on("close", () => {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    saveBounds(window);
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

  // Don't let in-page link clicks (e.g. in rendered markdown) navigate the
  // window away from the app shell. http/https goes to the OS browser;
  // anything else (file://, mailto:, junk) is simply blocked.
  window.webContents.on("will-navigate", (event, url) => {
    event.preventDefault();
    if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
  });
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
    return { action: "deny" };
  });

  void window.loadFile(resolve(app.getAppPath(), "public", "index.html"));
  if (openDevTools) {
    window.webContents.openDevTools({ mode: "detach" });
  }
  return window;
}

function registerIpc(currentRuntime: ElectronRuntime) {
  handle("oxplow:getCurrentStream", () => currentRuntime.getCurrentStream());
  handle("oxplow:listStreams", () => currentRuntime.listStreams());
  handle("oxplow:switchStream", (_event, id: string) => currentRuntime.switchStream(id));
  handle("oxplow:renameCurrentStream", (_event, title: string) => currentRuntime.renameCurrentStream(title));
  handle("oxplow:renameStream", (_event, streamId: string, title: string) => currentRuntime.renameStream(streamId, title));
  handle("oxplow:getConfig", () => currentRuntime.getConfig());
  handle("oxplow:setAgentPromptAppend", (_event, text: string) => currentRuntime.setAgentPromptAppend(text));
  handle("oxplow:setSnapshotRetentionDays", (_event, days: number) => currentRuntime.setSnapshotRetentionDays(days));
  handle("oxplow:setSnapshotMaxFileBytes", (_event, bytes: number) => currentRuntime.setSnapshotMaxFileBytes(bytes));
  handle("oxplow:setGeneratedDirs", (_event, dirs: string[]) => currentRuntime.setGeneratedDirs(dirs));
  handle("oxplow:listBranches", () => currentRuntime.listBranches());
  handle("oxplow:listGitRefs", () => currentRuntime.listGitRefs());
  handle("oxplow:renameGitBranch", (_event, from: string, to: string) => currentRuntime.renameGitBranch(from, to));
  handle("oxplow:deleteGitBranch", (_event, branch: string, options?: { force?: boolean }) =>
    currentRuntime.deleteGitBranch(branch, options),
  );
  handle("oxplow:gitMergeInto", (_event, streamId: string, other: string) =>
    currentRuntime.gitMergeInto(streamId, other),
  );
  handle("oxplow:gitRebaseOnto", (_event, streamId: string, onto: string) =>
    currentRuntime.gitRebaseOnto(streamId, onto),
  );
  handle("oxplow:getWorkspaceContext", () => currentRuntime.getWorkspaceContext());
  handle("oxplow:createStream", (_event, input) => currentRuntime.createStream(input));
  handle("oxplow:listAdoptableWorktrees", () => currentRuntime.listAdoptableWorktrees());
  handle("oxplow:checkoutStreamBranch", (_event, streamId: string, branch: string) =>
    currentRuntime.checkoutStreamBranch(streamId, branch),
  );
  handle("oxplow:getThreadState", (_event, streamId: string) => currentRuntime.getThreadState(streamId));
  handle("oxplow:createThread", (_event, streamId: string, title: string) => currentRuntime.createThread(streamId, title));
  handle("oxplow:reorderThread", (_event, streamId: string, threadId: string, targetIndex: number) => currentRuntime.reorderThread(streamId, threadId, targetIndex));
  handle("oxplow:reorderThreads", (_event, streamId: string, orderedThreadIds: string[]) => currentRuntime.reorderThreads(streamId, orderedThreadIds));
  handle("oxplow:reorderStreams", (_event, orderedStreamIds: string[]) => currentRuntime.reorderStreams(orderedStreamIds));
  handle("oxplow:selectThread", (_event, streamId: string, threadId: string) => currentRuntime.selectThread(streamId, threadId));
  handle("oxplow:promoteThread", (_event, streamId: string, threadId: string) => currentRuntime.promoteThread(streamId, threadId));
  handle("oxplow:completeThread", (_event, streamId: string, threadId: string) => currentRuntime.completeThread(streamId, threadId));
  handle("oxplow:renameThread", (_event, streamId: string, threadId: string, title: string) => currentRuntime.renameThread(streamId, threadId, title));
  handle("oxplow:setAutoCommit", (_event, streamId: string, threadId: string, enabled: boolean) => currentRuntime.setAutoCommit(streamId, threadId, enabled));
  handle("oxplow:setStreamPrompt", (_event, streamId: string, prompt: string | null) => currentRuntime.setStreamPrompt(streamId, prompt));
  handle("oxplow:setThreadPrompt", (_event, streamId: string, threadId: string, prompt: string | null) => currentRuntime.setThreadPrompt(streamId, threadId, prompt));
  handle("oxplow:getThreadWorkState", (_event, streamId: string, threadId: string) => currentRuntime.workItemApi.getThreadWorkState(streamId, threadId));
  handle("oxplow:createWorkItem", (_event, streamId: string, threadId: string, input) => currentRuntime.workItemApi.createWorkItem(streamId, threadId, input));
  handle("oxplow:updateWorkItem", (_event, streamId: string, threadId: string, itemId: string, changes) => currentRuntime.workItemApi.updateWorkItem(streamId, threadId, itemId, changes));
  handle("oxplow:deleteWorkItem", (_event, streamId: string, threadId: string, itemId: string) => currentRuntime.workItemApi.deleteWorkItem(streamId, threadId, itemId));
  handle("oxplow:reorderWorkItems", (_event, streamId: string, threadId: string, orderedItemIds: string[]) => currentRuntime.workItemApi.reorderWorkItems(streamId, threadId, orderedItemIds));
  handle("oxplow:moveWorkItemToThread", (_event, streamId: string, fromThreadId: string, itemId: string, toThreadId: string, toStreamId?: string) => currentRuntime.workItemApi.moveWorkItemToThread(streamId, fromThreadId, itemId, toThreadId, toStreamId));
  handle("oxplow:getBacklogState", () => currentRuntime.workItemApi.getBacklogState());
  handle("oxplow:createBacklogItem", (_event, input) => currentRuntime.workItemApi.createBacklogItem(input));
  handle("oxplow:updateBacklogItem", (_event, itemId: string, changes) => currentRuntime.workItemApi.updateBacklogItem(itemId, changes));
  handle("oxplow:deleteBacklogItem", (_event, itemId: string) => currentRuntime.workItemApi.deleteBacklogItem(itemId));
  handle("oxplow:reorderBacklog", (_event, orderedItemIds: string[]) => currentRuntime.workItemApi.reorderBacklog(orderedItemIds));
  handle("oxplow:moveWorkItemToBacklog", (_event, streamId: string, fromThreadId: string, itemId: string) => currentRuntime.workItemApi.moveWorkItemToBacklog(streamId, fromThreadId, itemId));
  handle("oxplow:moveBacklogItemToThread", (_event, streamId: string, itemId: string, toThreadId: string) => currentRuntime.workItemApi.moveBacklogItemToThread(streamId, itemId, toThreadId));
  handle("oxplow:getGitLog", (_event, streamId: string, options?: { limit?: number }) => currentRuntime.getGitLog(streamId, options));
  handle("oxplow:getCommitDetail", (_event, streamId: string, sha: string) => currentRuntime.getCommitDetail(streamId, sha));
  handle("oxplow:getChangeScopes", (_event, streamId: string) => currentRuntime.getChangeScopes(streamId));
  handle("oxplow:searchWorkspaceText", (_event, streamId: string, query: string, options?: { limit?: number }) => currentRuntime.searchWorkspaceText(streamId, query, options));
  handle("oxplow:gitRestorePath", (_event, streamId: string, path: string) => currentRuntime.gitRestorePath(streamId, path));
  handle("oxplow:gitAddPath", (_event, streamId: string, path: string) => currentRuntime.gitAddPath(streamId, path));
  handle("oxplow:gitAppendToGitignore", (_event, streamId: string, path: string) => currentRuntime.gitAppendToGitignore(streamId, path));
  handle("oxplow:gitPush", (_event, streamId: string, options) => currentRuntime.gitPush(streamId, options));
  handle("oxplow:gitPull", (_event, streamId: string, options) => currentRuntime.gitPull(streamId, options));
  handle("oxplow:gitCommitAll", (_event, streamId: string, message: string, options) => currentRuntime.gitCommitAll(streamId, message, options));
  handle("oxplow:listFileCommits", (_event, streamId: string, path: string, limit?: number) => currentRuntime.listFileCommits(streamId, path, limit));
  handle("oxplow:gitBlame", (_event, streamId: string, path: string) => currentRuntime.gitBlame(streamId, path));
  handle("oxplow:localBlame", (_event, streamId: string, path: string) => currentRuntime.localBlame(streamId, path));
  handle("oxplow:listAllRefs", (_event, streamId: string) => currentRuntime.listAllRefs(streamId));
  handle("oxplow:addWorkItemNote", (_event, streamId: string, threadId: string, itemId: string, note: string) => currentRuntime.workItemApi.addWorkItemNote(streamId, threadId, itemId, note));
  handle("oxplow:listWorkItemEvents", (_event, streamId: string, threadId: string, itemId?: string) => currentRuntime.workItemApi.listWorkItemEvents(streamId, threadId, itemId));
  handle("oxplow:getWorkNotes", (_event, itemId: string) => currentRuntime.workItemApi.getWorkNotes(itemId));
  handle("oxplow:listAgentTurns", (_event, streamId: string, threadId: string, limit?: number) => currentRuntime.workItemApi.listAgentTurns(streamId, threadId, limit));
  handle("oxplow:listOpenTurns", (_event, threadId: string) => currentRuntime.listOpenTurns(threadId));
  handle("oxplow:listRecentInactiveTurns", (_event, threadId: string, limit?: number) => currentRuntime.listRecentInactiveTurns(threadId, limit));
  handle("oxplow:archiveAgentTurn", (_event, turnId: string) => currentRuntime.archiveAgentTurn(turnId));
  handle("oxplow:listWorkItemEfforts", (_event, itemId: string) => currentRuntime.workItemApi.listWorkItemEfforts(itemId));
  handle("oxplow:listSnapshots", (_event, streamId: string, limit?: number) => currentRuntime.listSnapshots(streamId, limit));
  handle("oxplow:getSnapshotSummary", (_event, snapshotId: string, previousSnapshotId?: string | null) => currentRuntime.getSnapshotSummary(snapshotId, previousSnapshotId));
  handle("oxplow:getSnapshotPairDiff", (_event, beforeSnapshotId: string | null, afterSnapshotId: string, path: string) => currentRuntime.getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path));
  handle("oxplow:getEffortFiles", (_event, effortId: string) => currentRuntime.getEffortFiles(effortId));
  handle("oxplow:listEffortsEndingAtSnapshots", (_event, snapshotIds: string[]) => currentRuntime.listEffortsEndingAtSnapshots(snapshotIds));
  handle("oxplow:restoreFileFromSnapshot", (_event, streamId: string, snapshotId: string, path: string) => currentRuntime.restoreFileFromSnapshot(streamId, snapshotId, path));
  handle("oxplow:getBranchChanges", (_event, streamId: string, baseRef?: string) => currentRuntime.getBranchChanges(streamId, baseRef));
  handle("oxplow:readFileAtRef", (_event, streamId: string, ref: string, path: string) => currentRuntime.readFileAtRef(streamId, ref, path));
  handle("oxplow:listWorkspaceEntries", (_event, streamId: string, path?: string) => currentRuntime.listWorkspaceEntries(streamId, path));
  handle("oxplow:listWorkspaceFiles", (_event, streamId: string) => currentRuntime.listWorkspaceFiles(streamId));
  handle("oxplow:readWorkspaceFile", (_event, streamId: string, path: string) => currentRuntime.readWorkspaceFile(streamId, path));
  handle("oxplow:writeWorkspaceFile", (_event, streamId: string, path: string, content: string) => currentRuntime.writeWorkspaceFile(streamId, path, content));
  handle("oxplow:createWorkspaceFile", (_event, streamId: string, path: string, content?: string) => currentRuntime.createWorkspaceFile(streamId, path, content));
  handle("oxplow:createWorkspaceDirectory", (_event, streamId: string, path: string) => currentRuntime.createWorkspaceDirectory(streamId, path));
  handle("oxplow:renameWorkspacePath", (_event, streamId: string, fromPath: string, toPath: string) => currentRuntime.renameWorkspacePath(streamId, fromPath, toPath));
  handle("oxplow:deleteWorkspacePath", (_event, streamId: string, path: string) => currentRuntime.deleteWorkspacePath(streamId, path));
  handle("oxplow:listWikiNotes", (_event, streamId: string) => currentRuntime.listWikiNotes(streamId));
  handle("oxplow:readWikiNoteBody", (_event, streamId: string, slug: string) => currentRuntime.readWikiNoteBody(streamId, slug));
  handle("oxplow:writeWikiNoteBody", (_event, streamId: string, slug: string, body: string) => currentRuntime.writeWikiNoteBody(streamId, slug, body));
  handle("oxplow:deleteWikiNote", (_event, streamId: string, slug: string) => currentRuntime.deleteWikiNote(streamId, slug));
  handle("oxplow:listCommitPoints", (_event, threadId: string) => currentRuntime.listCommitPoints(threadId));
  handle("oxplow:createCommitPoint", (_event, streamId: string, threadId: string) => currentRuntime.createCommitPoint(streamId, threadId));
  handle("oxplow:deleteCommitPoint", (_event, id: string) => currentRuntime.deleteCommitPoint(id));
  handle("oxplow:updateCommitPoint", (_event, id: string, changes: { mode?: "auto" | "approve" }) => currentRuntime.updateCommitPoint(id, changes));
  handle("oxplow:commitCommitPoint", (_event, id: string, message: string) => currentRuntime.commitCommitPoint(id, message));
  handle("oxplow:reorderThreadQueue", (_event, streamId: string, threadId: string, entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>) => currentRuntime.reorderThreadQueue(streamId, threadId, entries));
  handle("oxplow:listWaitPoints", (_event, threadId: string) => currentRuntime.listWaitPoints(threadId));
  handle("oxplow:createWaitPoint", (_event, streamId: string, threadId: string, note?: string | null) => currentRuntime.createWaitPoint(streamId, threadId, note));
  handle("oxplow:setWaitPointNote", (_event, id: string, note: string | null) => currentRuntime.setWaitPointNote(id, note));
  handle("oxplow:deleteWaitPoint", (_event, id: string) => currentRuntime.deleteWaitPoint(id));
  handle("oxplow:listHookEvents", (_event, streamId?: string) => currentRuntime.listHookEvents(streamId));
  handle("oxplow:listAgentStatuses", (_event, streamId?: string) => currentRuntime.listAgentStatuses(streamId));
  handle("oxplow:ping", () => currentRuntime.ping());
  handle("oxplow:logUi", (_event, payload: UiLogPayload) => currentRuntime.logUi(payload));
  handle("oxplow:updateEditorFocus", (_event, payload: EditorFocusPayload) => currentRuntime.updateEditorFocus(payload));
  handle("oxplow:setNativeMenu", (_event, groups: MenuGroupSnapshot[]) => {
    Menu.setApplicationMenu(buildNativeMenu(groups));
  });
  handle("oxplow:openTerminalSession", (_event, paneTarget: string, cols: number, rows: number, mode: "direct" | "tmux" = "direct") =>
    currentRuntime.openTerminalSession(paneTarget, cols, rows, mode, (sessionId, message) => {
      const payload: TerminalEvent = { sessionId, message };
      broadcast("oxplow:terminal-event", payload);
    }),
  );
  handle("oxplow:sendTerminalMessage", (_event, sessionId: string, message: string) =>
    currentRuntime.sendTerminalMessage(sessionId, message),
  );
  handle("oxplow:closeTerminalSession", (_event, sessionId: string) =>
    currentRuntime.closeTerminalSession(sessionId),
  );
  handle("oxplow:openLspClient", (_event, streamId: string, languageId: string) =>
    currentRuntime.openLspClient(streamId, languageId, (clientId, message) => {
      const payload: LspEvent = { clientId, message };
      broadcast("oxplow:lsp-event", payload);
    }),
  );
  handle("oxplow:sendLspMessage", (_event, clientId: string, message: string) =>
    currentRuntime.sendLspMessage(clientId, message),
  );
  handle("oxplow:closeLspClient", (_event, clientId: string) =>
    currentRuntime.closeLspClient(clientId),
  );
}

function broadcast(
  channel: "oxplow:event" | "oxplow:terminal-event" | "oxplow:lsp-event",
  payload: OxplowEvent | TerminalEvent | LspEvent,
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
    targetWindow.webContents.send("oxplow:menu-command", commandId);
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
  // Tear down IPC handlers first so any in-flight renderer requests
  // surface as a "no handler registered" rejection instead of crashing
  // the runtime mid-shutdown with "database is not open".
  for (const channel of registeredIpcChannels) {
    try { ipcMain.removeHandler(channel); } catch { /* ignore */ }
  }
  registeredIpcChannels.length = 0;
  if (currentRuntime) {
    await currentRuntime.dispose();
  }
}

type LockResult =
  | { ok: true; lockPath: string }
  | { ok: false; pid: number; lockPath: string };

/**
 * Claims an exclusive per-project lock at `.oxplow/runtime/instance.lock`
 * containing this process's PID. Refuses to start if another live process
 * already holds the lock; reclaims stale locks whose PID no longer exists.
 * Per-project means two different projects can each have their own oxplow.
 */
function acquireProjectLock(projectDir: string): LockResult {
  const runtimeDir = join(projectDir, ".oxplow", "runtime");
  mkdirSync(runtimeDir, { recursive: true });
  const lockPath = join(runtimeDir, "instance.lock");
  if (existsSync(lockPath)) {
    const priorPid = readLockPid(lockPath);
    if (priorPid != null && priorPid !== process.pid && isPidAlive(priorPid)) {
      return { ok: false, pid: priorPid, lockPath };
    }
    // stale — either the writer died without cleaning up, or it's our own pid
    try { unlinkSync(lockPath); } catch (err) {
      console.warn("[oxplow] could not remove stale lock file", lockPath, err);
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
    console.warn("[oxplow] could not release project lock", instanceLockPath, err);
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
