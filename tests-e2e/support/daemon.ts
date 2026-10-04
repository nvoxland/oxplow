// One test daemon: `oxplow-daemon-sim` over a throwaway git project with its
// own global config (`OXPLOW_HOME`) and tmux socket dir, so nothing reaches
// the person's real config, keychain or tmux server.
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

export async function startDaemon(): Promise<Daemon> {
  const bin = process.env.OXPLOW_E2E_DAEMON;
  const acpFake = process.env.OXPLOW_E2E_ACP_FAKE;
  if (!bin || !acpFake) throw new Error("global setup didn't build the daemon (OXPLOW_E2E_DAEMON)");
  const dir = mkdtempSync(join(tmpdir(), "oxplow-e2e-"));
  const project = join(dir, "project");
  const home = join(dir, "home");
  const tmux = join(dir, "tmux");
  for (const d of [project, home, tmux, join(project, ".oxplow")]) mkdirSync(d, { recursive: true });
  writeFileSync(join(project, ".oxplow", "project.yaml"), projectYaml(acpFake));
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
