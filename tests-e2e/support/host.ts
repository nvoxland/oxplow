// One test host: `oxplow-host` over its own home, running the simulation
// daemon, serving the built UI (`apps/desktop/dist`) at its own origin.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";

import { isolated, type Calls } from "./daemon.js";

export type Host = {
  /** Where the host listens (`http://127.0.0.1:<port>`). */
  base: string;
  /** The owner's token (this machine; the Mac app holds it). A device's
   *  page has none: it pairs. */
  token: string;
  /** Register a throwaway git project called `name`; its id. */
  addProject(name: string): string;
  /** Hold `project` for a device and wait until its daemon runs. */
  hold(project: string, device?: string): Promise<void>;
  /** A pairing code, as the owner makes one (Devices… on the Mac). */
  pairingCode(): Promise<string>;
  /** The paired devices. */
  devices(): Promise<Array<{ id: string; name: string; kind: string }>>;
  /** Remove a paired device, as its owner. */
  removeDevice(id: string): Promise<void>;
  /** Where `project`'s calls go through the host, as its owner: for
   *  `open(page, …)` on a host-served window. */
  project(project: string): Calls;
  /** The host's status: who is connected now. */
  status(): Promise<{ connected: Array<{ device: string; name: string }> }>;
  stop(): Promise<void>;
};

const DIST = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "apps", "desktop", "dist");

export async function startHost(): Promise<Host> {
  const bin = process.env.OXPLOW_E2E_HOST;
  const daemon = process.env.OXPLOW_E2E_DAEMON;
  const harnessFake = process.env.OXPLOW_E2E_HARNESS_FAKE;
  if (!bin || !daemon || !harnessFake) throw new Error("global setup didn't build the suite's binaries");
  const dir = mkdtempSync(join(tmpdir(), "oxplow-e2e-host-"));
  const env = isolated(dir, harnessFake);
  for (const d of [env.HOME!, env.OXPLOW_HOME!]) mkdirSync(d, { recursive: true });
  const token = randomUUID();
  const child: ChildProcess = spawn(bin, ["serve", "--bind", "127.0.0.1:0", "--token-stdin", "--daemon", daemon, "--ui", DIST], {
    env,
    stdio: ["pipe", "pipe", "inherit"],
    // Its own group: the daemons it starts end with it.
    detached: true,
  });
  const exited = new Promise<void>((r) => child.once("exit", () => r()));
  const stop = async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
    await exited;
    try {
      process.kill(-child.pid!, "SIGKILL");
    } catch {
      // Nothing left in the group.
    }
    rmSync(dir, { recursive: true, force: true });
  };
  try {
    child.stdin?.on("error", () => {});
    child.stdin?.write(`${token}\n`);
    const base = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("the host didn't start")), 60_000);
      createInterface({ input: child.stdout! }).on("line", (line) => {
        const m = /listening on (http:\/\/\S+)/.exec(line);
        if (m) {
          clearTimeout(timer);
          resolve(m[1]!);
        }
      });
      child.once("close", (code) => {
        clearTimeout(timer);
        reject(new Error(`the host exited (${code})`));
      });
    });
    const call = async (method: string, path: string, body?: unknown) => {
      const res = await fetch(`${base}${path}`, {
        method,
        headers: { Authorization: `Bearer ${token}`, "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${await res.text()}`);
      return res.json() as Promise<unknown>;
    };
    return {
      base,
      token,
      addProject(name) {
        const project = join(dir, name);
        mkdirSync(join(project, ".oxplow"), { recursive: true });
        writeFileSync(
          join(project, ".oxplow", "project.yaml"),
          "oxplow.agents.claude.enabled: false\noxplow.agents.codex.enabled: false\noxplow.agents.opencode.enabled: false\n",
        );
        const git = (...args: string[]) => execFileSync("git", args, { cwd: project, env, stdio: "ignore" });
        git("init", "-q");
        git("commit", "-q", "--allow-empty", "-m", "init");
        const out = execFileSync(bin, ["project", "add", project], { env, encoding: "utf8" });
        const id = /^(prj-\w+)/.exec(out)?.[1];
        if (!id) throw new Error(`no project id in: ${out}`);
        return id;
      },
      async hold(project, device = "e2e-device") {
        await call("POST", "/host/holds", { device, project });
        const until = Date.now() + 120_000;
        for (;;) {
          const projects = (await call("GET", "/host/projects")) as Array<{ id: string; state: string; message?: string }>;
          const mine = projects.find((p) => p.id === project);
          if (mine?.state === "running") return;
          if (mine?.state === "crashed") throw new Error(`the daemon crashed: ${mine.message ?? ""}`);
          if (Date.now() > until) throw new Error(`the daemon never ran (${mine?.state})`);
          await new Promise((r) => setTimeout(r, 200));
        }
      },
      async pairingCode() {
        return ((await call("POST", "/host/pairing")) as { code: string }).code;
      },
      async devices() {
        return (await call("GET", "/host/devices")) as Array<{ id: string; name: string; kind: string }>;
      },
      async removeDevice(id) {
        await call("DELETE", `/host/devices/${id}`);
      },
      project(project) {
        return { base: `${base}/p/${project}`, token };
      },
      async status() {
        return (await call("GET", "/host/status")) as { connected: Array<{ device: string; name: string }> };
      },
      stop,
    };
  } catch (e) {
    await stop();
    throw e;
  }
}
