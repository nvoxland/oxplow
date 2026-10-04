// One test daemon: `oxplow-daemon-sim` over a throwaway git project with its
// own global config (`OXPLOW_HOME`) and tmux socket dir, so nothing reaches
// the person's real config, keychain or tmux server.
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { chmodSync, cpSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
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
  stop(): Promise<void>;
};

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

export async function startDaemon(): Promise<Daemon> {
  const bin = process.env.OXPLOW_E2E_DAEMON;
  const acpFake = process.env.OXPLOW_E2E_ACP_FAKE;
  const providerFake = process.env.OXPLOW_E2E_PROVIDER_FAKE;
  if (!bin || !acpFake || !providerFake) throw new Error("global setup didn't build the suite's binaries");
  const dir = mkdtempSync(join(tmpdir(), "oxplow-e2e-"));
  const project = join(dir, "project");
  const home = join(dir, "home");
  const tmux = join(dir, "tmux");
  for (const d of [project, home, tmux, join(project, ".oxplow")]) mkdirSync(d, { recursive: true });
  writeFileSync(join(project, ".oxplow", "project.yaml"), projectYaml(acpFake));
  installTestExtension(project, providerFake);
  const git = (...args: string[]) => execFileSync("git", args, { cwd: project, stdio: "ignore" });
  git("init", "-q");
  git("-c", "user.name=e2e", "-c", "user.email=e2e@example.com", "commit", "-q", "--allow-empty", "-m", "init");

  const token = randomUUID();
  const child: ChildProcess = spawn(bin, ["--project", project, "--bind", "127.0.0.1:0", "--token-stdin"], {
    env: { ...process.env, OXPLOW_HOME: home, TMUX_TMPDIR: tmux, RUST_LOG: process.env.RUST_LOG ?? "warn" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stderr: string[] = [];
  child.stderr?.on("data", (b: Buffer) => stderr.push(b.toString()));
  child.stdin?.end(`${token}\n`);
  const base = await new Promise<string>((resolve, reject) => {
    const lines = createInterface({ input: child.stdout! });
    const timer = setTimeout(() => reject(new Error(`the daemon didn't start:\n${stderr.join("")}`)), 120_000);
    lines.on("line", (line) => {
      const m = /listening on (http:\/\/\S+)/.exec(line);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]!);
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`the daemon exited (${code}):\n${stderr.join("")}`));
    });
  });
  return {
    base,
    token,
    project,
    async stop() {
      if (child.exitCode === null) {
        const exited = new Promise((r) => child.once("exit", r));
        child.kill("SIGTERM");
        await exited;
      }
      rmSync(dir, { recursive: true, force: true });
    },
  };
}

/** Call `/ipc/<name>` as the person; the result's data, or a thrown error. */
export async function ipc<T = unknown>(daemon: Daemon, name: string, args: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch(`${daemon.base}/ipc/${name}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${daemon.token}`, "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  const reply = (await res.json()) as { status: string; data?: T; error?: unknown };
  if (reply.status !== "ok") throw new Error(`${name}: ${JSON.stringify(reply.error)}`);
  return reply.data as T;
}

/** Run a bus command as the person, confirmed. */
export function run<T = unknown>(daemon: Daemon, name: string, input: Record<string, unknown>): Promise<T> {
  return ipc<T>(daemon, "run_command", { name, input, confirmed: true });
}

/** Poll `check` every 200 ms until it holds; throw `what` after `ms`. For
 *  daemon state a write settles into in the background (boot's tasks, the
 *  search index), never for the page — specs wait on the page with
 *  web-first `expect`. */
export async function until(what: string, ms: number, check: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + ms;
  while (!(await check())) {
    if (Date.now() > deadline) throw new Error(`timed out waiting: ${what}`);
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
 *  the write, so the event can't be missed. */
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

type SearchHit = { kind: string; title: string };

/** Wait until site search finds `query` among `kind` hits (`task`,
 *  `wiki`): the index is built in the background after a write. */
export async function searchable(daemon: Daemon, query: string, kind: string): Promise<void> {
  await until(`search to index "${query}" (${kind})`, 30_000, async () => {
    const hits = await ipc<SearchHit[]>(daemon, "search", { query, streamId: null, kinds: [kind], limit: 10 });
    return hits.length > 0;
  });
}
