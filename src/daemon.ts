import { basename, resolve } from "node:path";
import { StreamStore } from "./stream-store.js";
import { detectCurrentBranch, isGitRepo } from "./git.js";
import { ensureStreamSession } from "./fleet.js";
import { startServer } from "./server.js";
import { createSessionFiles, destroySessionFiles, type SessionFiles } from "./session-files.js";
import { startMcpServer, type McpServerHandle } from "./mcp-server.js";
import { HookEventStore } from "./hook-ingest.js";
import { killSession, watchSession } from "./tmux.js";

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
      projectBase,
    });
    console.log(`[newde] created stream ${stream.id} for branch '${branch}'`);
  } else {
    console.log(`[newde] reusing stream ${stream.id} for branch '${branch}'`);
  }

  const sessionFiles: SessionFiles = createSessionFiles({ daemonPort: port });
  console.log(`[newde] session files at ${sessionFiles.dir}`);

  const hookEvents = new HookEventStore(1000);

  let mcp: McpServerHandle;
  try {
    mcp = await startMcpServer({
      workspaceFolders: [projectDir],
    });
    console.log(`[newde] mcp ws://127.0.0.1:${mcp.port} (lock ${mcp.lockfilePath})`);
  } catch (e) {
    console.warn(`[newde] failed to start mcp server:`, e);
    destroySessionFiles(sessionFiles);
    process.exit(1);
  }

  // Build the per-spawn claude command. --settings merges our hooks on top
  // of the user's real ~/.claude settings, so credentials/history/prefs stay
  // intact and there's no re-login.
  const claudeCommand = `claude --settings ${shellEscape(sessionFiles.settingsPath)}`;

  const shutdown = async () => {
    console.log(`[newde] shutting down`);
    if (tmuxSession) {
      killSession(tmuxSession);
      console.log(`[newde] killed tmux session '${tmuxSession}'`);
    }
    try { await mcp.stop(); } catch {}
    destroySessionFiles(sessionFiles);
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  let tmuxSession: string | null = null;
  try {
    ensureStreamSession(stream, projectDir);
    tmuxSession = stream.panes.working.split(":")[0];
    watchSession(tmuxSession, process.pid);
    console.log(`[newde] tmux session '${tmuxSession}' watched (sentinel pid monitors daemon ${process.pid})`);
  } catch (e) {
    console.warn(`[newde] failed to ensure tmux session:`, e);
  }

  const publicDir = resolve(new URL("..", import.meta.url).pathname, "public");

  startServer({
    store,
    currentStreamId: stream.id,
    publicDir,
    projectDir,
    port,
    claudeCommand,
    hookEvents,
  });
}

function shellEscape(s: string): string {
  // Wrap in single quotes, escaping any embedded single quotes.
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

main();
