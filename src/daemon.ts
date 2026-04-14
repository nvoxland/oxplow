import { basename, resolve } from "node:path";
import { StreamStore } from "./stream-store.js";
import { detectCurrentBranch, isGitRepo } from "./git.js";
import { ensureStreamSession } from "./fleet.js";
import { startServer } from "./server.js";

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

function main() {
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
    console.log(`[newde2] created stream ${stream.id} for branch '${branch}'`);
  } else {
    console.log(`[newde2] reusing stream ${stream.id} for branch '${branch}'`);
  }

  try {
    ensureStreamSession(stream, projectDir);
  } catch (e) {
    console.warn(`[newde2] failed to ensure tmux session:`, e);
  }

  const publicDir = resolve(new URL("..", import.meta.url).pathname, "public");

  startServer({
    store,
    currentStreamId: stream.id,
    publicDir,
    projectDir,
    port,
  });
}

main();
