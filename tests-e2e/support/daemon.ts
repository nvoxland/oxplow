// One test daemon: `oxplow-daemon-sim` over a throwaway git project with its
// own global config (`OXPLOW_HOME`), home, shell and git config, so nothing
// reaches the person's real config, keychain, rc files or shell history.
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { chmodSync, cpSync, createWriteStream, mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";

export type Daemon = {
  /** The daemon's HTTP base (`http://127.0.0.1:<port>`). */
  base: string;
  /** The UI token every `/ipc` call carries. */
  token: string;
  /** The project's worktree. */
  project: string;
  /** Its process. */
  pid: number;
  /** Its stderr, kept after it stops (`tests-e2e/.output/daemons/`). */
  log: string;
  stop(): Promise<void>;
};

/** Where every daemon's log is kept: global setup empties it per run, and
 *  CI uploads it with the traces when a spec fails. */
export const LOG_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", ".output", "daemons");

/** The project's `.oxplow/project.yaml`: its threads run the fake ACP agent,
 *  so no spec ever starts a real agent CLI. */
function projectYaml(acpFake: string): string {
  return `agents: [acp]\nacpAgents:\n  - { name: fake, command: ${JSON.stringify(acpFake)} }\n`;
}

/** Put the suite's test extension (`tests-e2e/fixtures/extension`) into
 *  `project`, with the wrapper that runs the fake provider this checkout
 *  built: its service's state in the project's `.oxplow/`, one file per
 *  instance, as the provider tests keep it. */
function installTestExtension(project: string, providerFake: string): void {
  const fixture = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures", "extension");
  const dir = join(project, "oxplow", "extensions", "e2e");
  cpSync(fixture, dir, { recursive: true });
  mkdirSync(join(dir, "bin"), { recursive: true });
  const state = join(project, ".oxplow");
  const script = join(dir, "bin", "provider");
  writeFileSync(
    script,
    `#!/bin/sh\nOXPLOW_FAKE_STATE="${state}/fake-state-$OXPLOW_PROVIDER_ID.json" exec ${JSON.stringify(providerFake)} "$@"\n`,
  );
  chmodSync(script, 0o755);
}

/** Put the documented github example (`examples/extensions/github`) into
 *  `project`, its `sync.sh` swapped for one that prints the suite's pull
 *  requests (`fixtures/github-prs.json`) — no GitHub, the same entity. */
function installGithubExample(project: string): void {
  const here = dirname(fileURLToPath(import.meta.url));
  const dir = join(project, "oxplow", "extensions", "github");
  cpSync(join(here, "..", "..", "examples", "extensions", "github"), dir, { recursive: true });
  cpSync(join(here, "..", "fixtures", "github-prs.json"), join(dir, "prs.json"));
  writeFileSync(join(dir, "sync.sh"), `#!/bin/sh\nexec cat "$(dirname "$0")/prs.json"\n`);
  chmodSync(join(dir, "sync.sh"), 0o755);
}

/** The environment a daemon runs in: its own home (so a terminal's shell
 *  reads no rc file and writes no history of the person's), a PATH without
 *  the person's own bin dirs, a plain `/bin/sh`, and no global or system
 *  git config. */
function isolated(dir: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    // The person's own bin dirs (an agent CLI installed under their home)
    // are out of reach: no spec ever starts a real agent CLI.
    PATH: (process.env.PATH ?? "")
      .split(":")
      .filter((d) => d && !d.startsWith(homedir()))
      .join(":"),
    HOME: join(dir, "user"),
    OXPLOW_HOME: join(dir, "home"),
    SHELL: "/bin/sh",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "e2e",
    GIT_AUTHOR_EMAIL: "e2e@example.com",
    GIT_COMMITTER_NAME: "e2e",
    GIT_COMMITTER_EMAIL: "e2e@example.com",
    RUST_LOG: process.env.RUST_LOG ?? "warn",
  };
  for (const name of ["ZDOTDIR", "BASH_ENV", "ENV", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"]) {
    delete env[name];
  }
  return env;
}

/** Start a daemon over a new project in a temp dir under `tmp`. Whatever
 *  goes wrong before it's listening, the process is killed and the dir
 *  removed; its log is kept and named in the error. `bin` and `tmp` are for
 *  the harness's own spec. */
export async function startDaemon({ bin = process.env.OXPLOW_E2E_DAEMON, tmp = tmpdir() }: { bin?: string; tmp?: string } = {}): Promise<Daemon> {
  const acpFake = process.env.OXPLOW_E2E_ACP_FAKE;
  const providerFake = process.env.OXPLOW_E2E_PROVIDER_FAKE;
  if (!bin || !acpFake || !providerFake) throw new Error("global setup didn't build the suite's binaries");
  const dir = mkdtempSync(join(tmp, "oxplow-e2e-"));
  mkdirSync(LOG_DIR, { recursive: true });
  const log = join(LOG_DIR, `${basename(dir)}.log`);
  const env = isolated(dir);
  const project = join(dir, "project");
  let child: ChildProcess | undefined;
  let exited: Promise<void> = Promise.resolve();
  const stop = async () => {
    if (child) {
      // `exit` fires once whether it ends by a code or a signal; listened
      // for from the spawn, so a daemon already dead can't hang this.
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
      await exited;
    }
    rmSync(dir, { recursive: true, force: true });
  };
  try {
    for (const d of [project, env.HOME!, env.OXPLOW_HOME!, join(project, ".oxplow")]) mkdirSync(d, { recursive: true });
    writeFileSync(join(project, ".oxplow", "project.yaml"), projectYaml(acpFake));
    installTestExtension(project, providerFake);
    installGithubExample(project);
    const git = (...args: string[]) => execFileSync("git", args, { cwd: project, env, stdio: "ignore" });
    git("init", "-q");
    git("commit", "-q", "--allow-empty", "-m", "init");

    const token = randomUUID();
    const spawned = spawn(bin, ["--project", project, "--bind", "127.0.0.1:0", "--token-stdin"], {
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    child = spawned;
    exited = new Promise((r) => spawned.once("exit", () => r()));
    const logFile = createWriteStream(log);
    const stderr: string[] = [];
    spawned.stderr?.on("data", (b: Buffer) => {
      stderr.push(b.toString());
      logFile.write(b);
    });
    spawned.once("close", () => logFile.end());
    // Stdin stays open: it's the daemon's lifeline, and closing it stops
    // the daemon (tsk1073).
    spawned.stdin?.write(`${token}\n`);
    const base = await new Promise<string>((resolve, reject) => {
      const said = () => `${stderr.join("")}\n(log: ${log})`;
      const lines = createInterface({ input: spawned.stdout! });
      const timer = setTimeout(() => reject(new Error(`the daemon didn't start:\n${said()}`)), 120_000);
      lines.on("line", (line) => {
        const m = /listening on (http:\/\/\S+)/.exec(line);
        if (m) {
          clearTimeout(timer);
          resolve(m[1]!);
        }
      });
      spawned.once("error", (e) => {
        clearTimeout(timer);
        reject(new Error(`the daemon didn't spawn: ${e.message}\n(log: ${log})`));
      });
      // `close`, not `exit`: by then all it wrote to stderr has been read.
      spawned.once("close", (code, signal) => {
        clearTimeout(timer);
        reject(new Error(`the daemon exited (${code ?? signal}):\n${said()}`));
      });
    });
    return { base, token, project, pid: spawned.pid!, log, stop };
  } catch (e) {
    await stop();
    throw e;
  }
}

/** Call `/ipc/<name>` as the person; the result's data, or a thrown error. */
export async function ipc<T = unknown>(daemon: Daemon, name: string, args: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch(`${daemon.base}/ipc/${name}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${daemon.token}`, "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const text = await res.text();
  let reply: { status: string; data?: T; error?: unknown };
  try {
    reply = JSON.parse(text) as typeof reply;
  } catch {
    throw new Error(`${name}: HTTP ${res.status}, not a JSON reply: ${text.slice(0, 500)}`);
  }
  if (reply.status !== "ok") throw new Error(`${name}: HTTP ${res.status}: ${JSON.stringify(reply.error)}`);
  return reply.data as T;
}

/** Run a bus command as the person, confirmed. */
export function run<T = unknown>(daemon: Daemon, name: string, input: Record<string, unknown>): Promise<T> {
  return ipc<T>(daemon, "run_command", { id: name, input, confirmed: true });
}

/** Poll `check` every 200 ms until it holds; throw `what` after `ms`. A
 *  check that throws is tried again — the timeout names the last error —
 *  so a call the daemon isn't ready for yet doesn't end the wait. For
 *  daemon state a write settles into in the background (boot's tasks, the
 *  search index), never for the page — specs wait on the page with
 *  web-first `expect`. */
export async function until(what: string, ms: number, check: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + ms;
  let last: unknown = null;
  for (;;) {
    try {
      if (await check()) return;
      last = null;
    } catch (e) {
      last = e;
    }
    if (Date.now() > deadline) {
      const why = last === null ? "" : ` (last: ${last instanceof Error ? last.message : String(last)})`;
      throw new Error(`timed out waiting: ${what}${why}`);
    }
    await new Promise((r) => setTimeout(r, 200));
  }
}

/** Wait until the daemon's boot work is done: no background task left. */
export async function settle(daemon: Daemon): Promise<void> {
  await until("background tasks to finish", 120_000, async () => {
    const tasks = await ipc<unknown[]>(daemon, "list_background_tasks");
    return tasks.length === 0;
  });
}

/** Do `write`, and resolve with its result once the daemon has said every
 *  model in `models` changed (`modelsChanged`). The socket is open before
 *  the write, and the daemon subscribes before it answers the upgrade
 *  (tsk995), so the event can't be missed. */
export async function waitForModels<T>(daemon: Daemon, models: string[], write: () => Promise<T>): Promise<T> {
  const url = `${daemon.base.replace(/^http/, "ws")}/events?token=${encodeURIComponent(daemon.token)}`;
  const socket = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error(`couldn't open ${url}`)), { once: true });
  });
  const pending = new Set(models);
  const changed = new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no modelsChanged for ${[...pending].join(", ")}`)), 30_000);
    socket.addEventListener("message", (msg) => {
      const frame = JSON.parse(String(msg.data)) as { channel?: string; payload?: { kind?: string; models?: string[] } };
      if (frame.channel !== "oxplow" || frame.payload?.kind !== "modelsChanged") return;
      for (const m of frame.payload.models ?? []) pending.delete(m);
      if (pending.size === 0) {
        clearTimeout(timer);
        resolve();
      }
    });
  });
  try {
    const result = await write();
    await changed;
    return result;
  } finally {
    socket.close();
  }
}

type Program = { kind: string; name: string; version: string | null; approved: boolean };

/** Approve one of the project's programs (`<kind>`, `<name>`: `provider`,
 *  `e2e/fake`) as a person does, at the version it is now. */
export async function approveProgram(daemon: Daemon, kind: string, name: string): Promise<void> {
  const programs = await ipc<Program[]>(daemon, "list_project_programs");
  const program = programs.find((p) => p.kind === kind && p.name === name);
  if (!program) throw new Error(`no program ${kind}:${name}`);
  if (program.approved) return;
  await ipc(daemon, "approve_project_program", { kind, name, version: program.version });
}

type Collector = { owner: string; spec: { id: string }; approved: boolean; version: string | null };

/** Approve an extension's collector (`owner`/`id`) as a person does on
 *  Settings → Data, at the version listed now. */
export async function approveCollector(daemon: Daemon, owner: string, id: string): Promise<void> {
  const collectors = await ipc<Collector[]>(daemon, "list_collectors");
  const collector = collectors.find((c) => c.owner === owner && c.spec.id === id);
  if (!collector?.version) throw new Error(`no collector ${owner}/${id} to approve`);
  if (collector.approved) return;
  await ipc(daemon, "approve_collector", { owner, id, version: collector.version });
}

type SearchHit = { kind: string; title: string };

/** Wait until site search finds `query` among `kind` hits (`task`,
 *  `wiki`): the index is built in the background after a write. */
export async function searchable(daemon: Daemon, query: string, kind: string): Promise<void> {
  await until(`search to index "${query}" (${kind})`, 30_000, async () => {
    const hits = await ipc<SearchHit[]>(daemon, "search", { query, streamId: null, kinds: [kind], limit: 10 });
    return hits.length > 0;
  });
}
