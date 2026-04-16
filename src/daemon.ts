import { basename, resolve } from "node:path";
import { loadProjectConfig, type AgentKind, type NewdeConfig } from "./config.js";
import { StreamStore, type PaneKind, type Stream } from "./stream-store.js";
import { detectCurrentBranch, isGitRepo } from "./git.js";
import { createDaemonLogger } from "./logger.js";
import { startServer } from "./server.js";
import { createSessionFiles, destroySessionFiles, type SessionFiles } from "./session-files.js";
import { startMcpServer, type McpServerHandle } from "./mcp-server.js";
import { HookEventStore } from "./hook-ingest.js";
import { ResumeTracker } from "./resume-tracker.js";
import { LspSessionManager } from "./lsp.js";
import { killSession } from "./tmux.js";
import { WorkspaceWatcherRegistry } from "./workspace-watch.js";

function parseArgs(argv: string[]) {
  const args: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (next && !next.startsWith("--")) {
        args[key] = next;
        i++;
      } else {
        args[key] = "true";
      }
    }
  }
  return args;
}

export async function main() {
  const args = parseArgs(process.argv.slice(2));
  const projectDir = resolve(args.project ?? process.cwd());
  const port = Number(args.port ?? 7457);
  const projectBase = basename(projectDir).replace(/[^a-zA-Z0-9_-]/g, "_");
  const logger = createDaemonLogger(projectDir).child({ pid: process.pid, subsystem: "daemon" });
  let config: NewdeConfig;
  try {
    config = loadProjectConfig(projectDir, logger.child({ subsystem: "config" }));
  } catch (e) {
    logger.error("failed to load project config", { error: errorMessage(e), configPath: `${projectDir}/newde.yaml` });
    process.exit(1);
  }

  logger.info("starting daemon", { projectDir, port, projectBase, agent: config.agent });

  const store = new StreamStore(projectDir, logger.child({ subsystem: "stream-store" }));

  const gitWorkspace = isGitRepo(projectDir);
  const branch = gitWorkspace ? detectCurrentBranch(projectDir) ?? projectBase : projectBase;

  let stream = store.findByBranch(branch);
  if (!stream) {
    stream = store.create({
      title: branch,
      branch,
      branchRef: gitWorkspace ? `refs/heads/${branch}` : branch,
      branchSource: "local",
      worktreePath: projectDir,
      projectBase,
    });
    logger.info("created initial stream", { streamId: stream.id, branch });
  } else {
    logger.info("reusing initial stream", { streamId: stream.id, branch });
  }
  store.ensureCurrentStreamId(stream.id);
  cleanupSessions(store.list());
  logger.info("initialized current stream", { streamId: store.getCurrentStreamId() });

  const hookEvents = new HookEventStore(1000);
  const resumeTracker = new ResumeTracker();
  const lspManager = new LspSessionManager(logger.child({ subsystem: "lsp" }));
  const workspaceWatchers = new WorkspaceWatcherRegistry(logger.child({ subsystem: "workspace-watch" }));
  const paneSessionFiles = new Map<string, SessionFiles>();
  for (const existingStream of store.list()) {
    workspaceWatchers.ensureWatching(existingStream);
  }

  let mcp: McpServerHandle;
  try {
    mcp = await startMcpServer({
      workspaceFolders: store.list().map((s) => s.worktree_path),
      logger: logger.child({ subsystem: "mcp" }),
    });
    logger.info("started mcp server", { port: mcp.port, lockfilePath: mcp.lockfilePath });
  } catch (e) {
    logger.error("failed to start mcp server", { error: errorMessage(e) });
    process.exit(1);
  }

  const shutdown = async (signal?: string) => {
    logger.info("shutting down daemon", { signal });
    cleanupSessions(store.list());
    try { workspaceWatchers.dispose(); } catch {}
    try { await lspManager.dispose(); } catch {}
    try { await mcp.stop(); } catch {}
    for (const files of paneSessionFiles.values()) {
      destroySessionFiles(files);
    }
    process.exit(0);
  };
  process.on("SIGINT", () => void shutdown("SIGINT"));
  process.on("SIGTERM", () => void shutdown("SIGTERM"));
  process.on("uncaughtException", (error) => {
    logger.error("uncaught exception", { error: errorMessage(error), stack: error instanceof Error ? error.stack : undefined });
  });
  process.on("unhandledRejection", (reason) => {
    logger.error("unhandled rejection", { error: errorMessage(reason) });
  });

  const publicDir = resolve(new URL("..", import.meta.url).pathname, "public");

  startServer({
    store,
    publicDir,
    projectDir,
    projectBase,
    port,
    hookEvents,
    workspaceWatchers,
    logger: logger.child({ subsystem: "server" }),
    resumeTracker,
    lspManager,
    getAgentCommand: (targetStream, pane) => {
      if (config.agent === "claude") {
        const key = `${targetStream.id}:${pane}`;
        let files = paneSessionFiles.get(key);
        if (!files) {
          files = createSessionFiles({ daemonPort: port, streamId: targetStream.id, pane });
          paneSessionFiles.set(key, files);
          logger.info("created session files", {
            streamId: targetStream.id,
            pane,
            settingsPath: files.settingsPath,
          });
        }
        return buildAgentCommand(config.agent, targetStream, pane, files.settingsPath);
      }
      return buildAgentCommand(config.agent, targetStream, pane);
    },
  });
}

function cleanupSessions(streams: Stream[]) {
  const sessions = new Set(streams.map((stream) => stream.panes.working.split(":")[0]));
  for (const session of sessions) {
    killSession(session);
  }
}

export function buildAgentCommand(
  agent: AgentKind,
  stream: Stream,
  pane: PaneKind,
  settingsPath?: string,
  appendSystemPrompt?: string,
  mcpConfig?: string,
): string {
  const resumeSessionId = pane === "working"
    ? stream.resume.working_session_id
    : stream.resume.talking_session_id;
  return buildAgentCommandForSession(agent, stream.worktree_path, resumeSessionId, settingsPath, appendSystemPrompt, mcpConfig);
}

export function buildAgentCommandForSession(
  agent: AgentKind,
  cwd: string,
  resumeSessionId: string,
  settingsPath?: string,
  appendSystemPrompt?: string,
  mcpConfig?: string,
): string {
  if (agent === "copilot") {
    return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && exec copilot`)}`;
  }

  if (!settingsPath) {
    throw new Error("Claude agent requires a settingsPath");
  }
  const promptArg = appendSystemPrompt
    ? ` --append-system-prompt ${shellEscape(appendSystemPrompt)}`
    : "";
  const mcpArg = mcpConfig
    ? ` --mcp-config ${shellEscape(mcpConfig)} --strict-mcp-config`
    : "";
  const claudeBase = `claude --settings ${shellEscape(settingsPath)}${promptArg}${mcpArg}`;
  const freshClaude = `exec ${claudeBase}`;
  const command = resumeSessionId
    ? `${claudeBase} --resume ${shellEscape(resumeSessionId)} || { echo '[newde] saved resume id was stale; starting a fresh Claude session' >&2; ${freshClaude}; }`
    : freshClaude;
  return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && ${command}`)}`;
}

function shellEscape(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

if ((import.meta as { main?: boolean }).main) {
  void main();
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
