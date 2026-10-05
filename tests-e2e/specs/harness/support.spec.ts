// The suite's own helpers (tsk995): a harness that hides a failure, hangs
// or leaks makes every spec built on it lie.
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AddressInfo } from "node:net";

import { expect, test } from "@playwright/test";

import { ipc, startDaemon, until, type Daemon } from "../../support/daemon.js";
import { sleepsIn } from "../../support/global-setup.js";

test("until retries a check that throws, and a timeout says what it last saw", async () => {
  let calls = 0;
  await until("a flaky check", 5_000, async () => {
    calls += 1;
    if (calls < 3) throw new Error("not yet");
    return true;
  });
  expect(calls).toBe(3);
  await expect(
    until("a broken check", 300, async () => {
      throw new Error("the daemon said no");
    }),
  ).rejects.toThrow(/a broken check.*the daemon said no/s);
});

test("ipc on a reply that isn't JSON names the call, the status and what came back", async () => {
  const server = createServer((_, res) => {
    res.writeHead(502, { "content-type": "text/html" });
    res.end("<html>Bad Gateway</html>");
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  try {
    const { port } = server.address() as AddressInfo;
    const daemon = { base: `http://127.0.0.1:${port}`, token: "t" } as Daemon;
    await expect(ipc(daemon, "list_streams")).rejects.toThrow(/list_streams: HTTP 502.*Bad Gateway/s);
  } finally {
    server.close();
  }
});

test("a daemon that fails to start leaves no project behind, and its log is kept", async () => {
  const tmp = mkdtempSync(join(tmpdir(), "oxplow-e2e-harness-"));
  try {
    const bin = join(tmp, "broken-daemon");
    writeFileSync(bin, "#!/bin/sh\necho 'boom: no database' >&2\nexit 3\n");
    chmodSync(bin, 0o755);
    const projects = join(tmp, "projects");
    mkdirSync(projects);
    const failure = await startDaemon({ bin, tmp: projects }).then(
      () => null,
      (e: Error) => e.message,
    );
    expect(failure).toMatch(/boom: no database/);
    expect(readdirSync(projects)).toEqual([]);
    const log = /\(log: ([^)]+)\)/.exec(failure ?? "")?.[1];
    expect(log && existsSync(log), `the daemon's log is kept (${failure})`).toBeTruthy();
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});

test("stopping a daemon a signal already killed returns", async () => {
  test.setTimeout(180_000);
  const daemon = await startDaemon();
  process.kill(daemon.pid, "SIGKILL");
  await until("the daemon to die", 10_000, async () => {
    try {
      process.kill(daemon.pid, 0);
      return false;
    } catch {
      return true;
    }
  });
  await daemon.stop();
  expect(existsSync(daemon.project)).toBe(false);
});

test("the sleep ban sees a timer by any spelling", async () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-e2e-sleeps-"));
  try {
    // Built from parts, so this file isn't one the ban refuses.
    const timer = ["set", "Timeout"].join("");
    const wait = ["waitFor", "Timeout"].join("");
    writeFileSync(join(dir, "a.spec.ts"), `await new Promise((r) => ${timer}(r, 500));\n`);
    writeFileSync(join(dir, "b.spec.ts"), `const pause = page.${wait};\nawait pause.call(page, 500);\n`);
    writeFileSync(join(dir, "c.spec.ts"), `test.${timer}(60_000);\nawait expect(page.getByText("x")).toBeVisible();\n`);
    expect(sleepsIn(dir).map((p) => p.slice(dir.length + 1))).toEqual(["a.spec.ts", "b.spec.ts"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
