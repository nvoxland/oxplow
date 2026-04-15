import { basename, resolve } from "node:path";
import { StreamStore, type PaneKind, type Stream } from "./stream-store.js";
import { detectCurrentBranch, isGitRepo } from "./git.js";
import { createDaemonLogger } from "./logger.js";
import { startServer } from "./server.js";
import { createSessionFiles, destroySessionFiles, type SessionFiles } from "./session-files.js";
import { startMcpServer, type McpServerHandle } from "./mcp-server.js";
import { HookEventStore } from "./hook-ingest.js";
import { ResumeTracker } from "./resume-tracker.js";
import { killSession } from "./tmux.js";

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

  logger.info("starting daemon", { projectDir, port, projectBase });

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
  const paneSessionFiles = new Map<string, SessionFiles>();

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
    logger: logger.child({ subsystem: "server" }),
    resumeTracker,
    getClaudeCommand: (targetStream, pane) => {
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
      return buildClaudeCommand(targetStream, pane, files.settingsPath);
    },
  });
}

function cleanupSessions(streams: Stream[]) {
  const sessions = new Set(streams.map((stream) => stream.panes.working.split(":")[0]));
  for (const session of sessions) {
    killSession(session);
  }
}

export function buildClaudeCommand(stream: Stream, pane: PaneKind, settingsPath: string): string {
  const resumeSessionId = pane === "working"
    ? stream.resume.working_session_id
    : stream.resume.talking_session_id;
  const freshClaude = `exec claude --settings ${shellEscape(settingsPath)}`;
  const command = resumeSessionId
    ? `claude --resume ${shellEscape(resumeSessionId)} --settings ${shellEscape(settingsPath)} || { echo '[newde] resume failed, starting fresh' >&2; ${freshClaude}; }`
    : freshClaude;
  return `sh -lc ${shellEscape(`cd ${shellEscape(stream.worktree_path)} && ${command}`)}`;
}

function shellEscape(s: string): string {
  // Wrap in single quotes, escaping any embedded single quotes.
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

if (import.meta.main) {
  void main();
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
