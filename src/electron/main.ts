import { app, BrowserWindow, dialog, ipcMain, Menu, type MenuItemConstructorOptions } from "electron";
import { resolve } from "node:path";
import { ElectronRuntime } from "./runtime.js";
import type { CommandId, LspEvent, MenuGroupSnapshot, NewdeEvent, TerminalEvent, UiLogPayload } from "./ipc-contract.js";

let runtime: ElectronRuntime | null = null;
let mainWindow: BrowserWindow | null = null;
let quitting = false;
let disposed = false;

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

  try {
    runtime = await ElectronRuntime.create(projectDir);
  } catch (error) {
    dialog.showErrorBox(
      "newde failed to start",
      error instanceof Error ? error.message : String(error),
    );
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
  ipcMain.handle("newde:listBranches", () => currentRuntime.listBranches());
  ipcMain.handle("newde:getWorkspaceContext", () => currentRuntime.getWorkspaceContext());
  ipcMain.handle("newde:createStream", (_event, input) => currentRuntime.createStream(input));
  ipcMain.handle("newde:getBatchState", (_event, streamId: string) => currentRuntime.getBatchState(streamId));
  ipcMain.handle("newde:createBatch", (_event, streamId: string, title: string) => currentRuntime.createBatch(streamId, title));
  ipcMain.handle("newde:reorderBatch", (_event, streamId: string, batchId: string, targetIndex: number) => currentRuntime.reorderBatch(streamId, batchId, targetIndex));
  ipcMain.handle("newde:selectBatch", (_event, streamId: string, batchId: string) => currentRuntime.selectBatch(streamId, batchId));
  ipcMain.handle("newde:promoteBatch", (_event, streamId: string, batchId: string) => currentRuntime.promoteBatch(streamId, batchId));
  ipcMain.handle("newde:completeBatch", (_event, streamId: string, batchId: string) => currentRuntime.completeBatch(streamId, batchId));
  ipcMain.handle("newde:getBatchWorkState", (_event, streamId: string, batchId: string) => currentRuntime.workItemApi.getBatchWorkState(streamId, batchId));
  ipcMain.handle("newde:createWorkItem", (_event, streamId: string, batchId: string, input) => currentRuntime.workItemApi.createWorkItem(streamId, batchId, input));
  ipcMain.handle("newde:updateWorkItem", (_event, streamId: string, batchId: string, itemId: string, changes) => currentRuntime.workItemApi.updateWorkItem(streamId, batchId, itemId, changes));
  ipcMain.handle("newde:deleteWorkItem", (_event, streamId: string, batchId: string, itemId: string) => currentRuntime.workItemApi.deleteWorkItem(streamId, batchId, itemId));
  ipcMain.handle("newde:reorderWorkItems", (_event, streamId: string, batchId: string, orderedItemIds: string[]) => currentRuntime.workItemApi.reorderWorkItems(streamId, batchId, orderedItemIds));
  ipcMain.handle("newde:addWorkItemNote", (_event, streamId: string, batchId: string, itemId: string, note: string) => currentRuntime.workItemApi.addWorkItemNote(streamId, batchId, itemId, note));
  ipcMain.handle("newde:listWorkItemEvents", (_event, streamId: string, batchId: string, itemId?: string) => currentRuntime.workItemApi.listWorkItemEvents(streamId, batchId, itemId));
  ipcMain.handle("newde:listAgentTurns", (_event, streamId: string, batchId: string, limit?: number) => currentRuntime.workItemApi.listAgentTurns(streamId, batchId, limit));
  ipcMain.handle("newde:listBatchFileChanges", (_event, streamId: string, batchId: string, limit?: number) => currentRuntime.workItemApi.listFileChanges(streamId, batchId, limit));
  ipcMain.handle("newde:listWorkspaceEntries", (_event, streamId: string, path?: string) => currentRuntime.listWorkspaceEntries(streamId, path));
  ipcMain.handle("newde:listWorkspaceFiles", (_event, streamId: string) => currentRuntime.listWorkspaceFiles(streamId));
  ipcMain.handle("newde:readWorkspaceFile", (_event, streamId: string, path: string) => currentRuntime.readWorkspaceFile(streamId, path));
  ipcMain.handle("newde:writeWorkspaceFile", (_event, streamId: string, path: string, content: string) => currentRuntime.writeWorkspaceFile(streamId, path, content));
  ipcMain.handle("newde:createWorkspaceFile", (_event, streamId: string, path: string, content?: string) => currentRuntime.createWorkspaceFile(streamId, path, content));
  ipcMain.handle("newde:createWorkspaceDirectory", (_event, streamId: string, path: string) => currentRuntime.createWorkspaceDirectory(streamId, path));
  ipcMain.handle("newde:renameWorkspacePath", (_event, streamId: string, fromPath: string, toPath: string) => currentRuntime.renameWorkspacePath(streamId, fromPath, toPath));
  ipcMain.handle("newde:deleteWorkspacePath", (_event, streamId: string, path: string) => currentRuntime.deleteWorkspacePath(streamId, path));
  ipcMain.handle("newde:listHookEvents", (_event, streamId?: string) => currentRuntime.listHookEvents(streamId));
  ipcMain.handle("newde:listAgentStatuses", (_event, streamId?: string) => currentRuntime.listAgentStatuses(streamId));
  ipcMain.handle("newde:ping", () => currentRuntime.ping());
  ipcMain.handle("newde:logUi", (_event, payload: UiLogPayload) => currentRuntime.logUi(payload));
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
