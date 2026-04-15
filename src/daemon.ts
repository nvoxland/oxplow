import { basename, resolve } from "node:path";
import { StreamStore, type PaneKind, type Stream } from "./stream-store.js";
import { detectCurrentBranch, isGitRepo } from "./git.js";
import { startServer } from "./server.js";
import { createSessionFiles, destroySessionFiles, type SessionFiles } from "./session-files.js";
import { startMcpServer, type McpServerHandle } from "./mcp-server.js";
import { HookEventStore } from "./hook-ingest.js";
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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const projectDir = resolve(args.project ?? process.cwd());
  const port = Number(args.port ?? 7457);
  const projectBase = basename(projectDir).replace(/[^a-zA-Z0-9_-]/g, "_");

  const store = new StreamStore(projectDir);

  const branch = isGitRepo(projectDir) ? detectCurrentBranch(projectDir) ?? projectBase : projectBase;

  let stream = store.findByBranch(branch);
  if (!stream) {
    stream = store.create({
      title: branch,
      branch,
      branchRef: isGitRepo(projectDir) ? `refs/heads/${branch}` : branch,
      branchSource: "local",
      worktreePath: projectDir,
      projectBase,
    });
    console.log(`[newde] created stream ${stream.id} for branch '${branch}'`);
  } else {
    console.log(`[newde] reusing stream ${stream.id} for branch '${branch}'`);
  }
  store.ensureCurrentStreamId(stream.id);
  cleanupSessions(store.list());

  const hookEvents = new HookEventStore(1000);
  const paneSessionFiles = new Map<string, SessionFiles>();

  let mcp: McpServerHandle;
  try {
    mcp = await startMcpServer({
      workspaceFolders: store.list().map((s) => s.worktree_path),
    });
    console.log(`[newde] mcp ws://127.0.0.1:${mcp.port} (lock ${mcp.lockfilePath})`);
  } catch (e) {
    console.warn(`[newde] failed to start mcp server:`, e);
    process.exit(1);
  }

  const shutdown = async () => {
    console.log(`[newde] shutting down`);
    cleanupSessions(store.list());
    try { await mcp.stop(); } catch {}
    for (const files of paneSessionFiles.values()) {
      destroySessionFiles(files);
    }
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  const publicDir = resolve(new URL("..", import.meta.url).pathname, "public");

  startServer({
    store,
    publicDir,
    projectDir,
    projectBase,
    port,
    hookEvents,
    getClaudeCommand: (targetStream, pane) => {
      const key = `${targetStream.id}:${pane}`;
      let files = paneSessionFiles.get(key);
      if (!files) {
        files = createSessionFiles({ daemonPort: port, streamId: targetStream.id, pane });
        paneSessionFiles.set(key, files);
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

function buildClaudeCommand(stream: Stream, pane: PaneKind, settingsPath: string): string {
  const resumeSessionId = pane === "working"
    ? stream.resume.working_session_id
    : stream.resume.talking_session_id;
  const resumeFlag = resumeSessionId ? ` --resume ${shellEscape(resumeSessionId)}` : "";
  return `claude${resumeFlag} --settings ${shellEscape(settingsPath)}`;
}

function shellEscape(s: string): string {
  // Wrap in single quotes, escaping any embedded single quotes.
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

main();
