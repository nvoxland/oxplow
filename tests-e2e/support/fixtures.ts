// The suite's fixtures: a daemon per spec, a page that talks to it, and a
// guard that fails any spec whose page threw.
import { cpSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { test as base, expect, type Page } from "@playwright/test";

import { deploy, ipc, settle, startDaemon, until, type Daemon } from "./daemon.js";

type Stream = { id: string };
type Thread = { id: string };

/** The stream and the thread its pages open on: the one a fresh stream is
 *  seeded with. It has no agent session; one opened on it runs the
 *  project's default agent — here the fake terminal harness (`agents:
 *  [scripted]`), so the suite never starts a real agent CLI. */
async function seededThread(daemon: Daemon): Promise<{ stream: string; thread: string }> {
  const [stream] = await ipc<Stream[]>(daemon, "list_streams");
  if (!stream) throw new Error("the daemon has no stream");
  const threads = await ipc<Thread[]>(daemon, "list_threads", { streamId: stream.id });
  const [seeded] = threads;
  if (threads.length !== 1 || !seeded) {
    throw new Error(`expected one seeded thread, got ${JSON.stringify(threads)}`);
  }
  return {
    stream: stream.id,
    thread: seeded.id,
  };
}

/** A worker's daemon, and the stream and thread its pages open on. */
export type Workspace = Daemon & { stream: string; thread: string };

/** A daemon over the workspace at `dir` (made there if it isn't yet),
 *  settled, its extensions deployed and pointed at the fake agent's thread;
 *  one that fails on the way is stopped, its dir removed. `keep` leaves the
 *  dir when it stops. */
async function workspace(dir: string, keep = false): Promise<Workspace> {
  const daemon = await startDaemon({ dir, keep });
  try {
    await settle(daemon);
    // The suite's extensions declare permissions, so boot waits for a
    // person to deploy them: deployed as they would, confirmed. A workspace
    // copied from the template has them deployed already.
    for (const name of ["e2e", "github"]) await deploy(daemon, name);
    // Extensions' datasets compile in the background after boot: the test
    // extension's is published once they are.
    await until("the extensions' datasets to publish", 60_000, async () => {
      const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT 1 FROM v_dataset WHERE view = 'v_e2e_item'",
      });
      return out.rows.length > 0;
    });
    // Its pages open on the fake harness, a provider from the project.
    await until("the fake harness to be registered", 60_000, async () => {
      const harnesses = await ipc<Array<{ id: string; enabled: boolean }>>(daemon, "list_agent_harnesses");
      return harnesses.some((h) => h.id === "scripted" && h.enabled);
    });
    return { ...daemon, ...(await seededThread(daemon)) };
  } catch (e) {
    await daemon.stop();
    throw e;
  }
}

/** Collect what `page` throws; check it once the spec is done. A spec that
 *  already failed (a timeout closes its page mid-request, which WebKit
 *  reports as failed fetches) is reported for its own error, not these. */
function guard(page: Page): () => void {
  const errors: string[] = [];
  // WebKit reports any request a reload or a closing page cuts short as "Fetch
  // API cannot load …/ipc/<name> due to access control checks", though no check
  // refused it. A request the daemon really refused fails the spec by its effect.
  const cutShort = /^Fetch API cannot load .*\/ipc\/\w+ due to access control checks\.$/;
  page.on("pageerror", (e) => {
    if (!cutShort.test(String(e))) errors.push(String(e));
  });
  return () => {
    if (test.info().status !== test.info().expectedStatus) return;
    expect(errors, "the page threw").toEqual([]);
  };
}

/** A page's storage pointing its transport at `base` with `token`. */
export function connectedTo(baseURL: string, base: string, token: string) {
  return {
    cookies: [],
    origins: [
      {
        origin: new URL(baseURL).origin,
        localStorage: [
          { name: "oxplow.remoteBase", value: base },
          { name: "oxplow.remoteToken", value: token },
        ],
      },
    ],
  };
}

/** One place a worker's tests run: the dir a test's workspace lives in, and
 *  the template copied into it first. The database and the store hold the
 *  dir's absolute path (the stream's worktree, each extension's runtime
 *  folder), so a template is only good at the path it was made at. */
type Slot = { dir: string; template: string };

/** Make a slot under `root`: boot a workspace from nothing at its dir (a git
 *  project, the store, the suite's extensions deployed, their datasets
 *  compiled), stop it, and keep a copy of the whole dir. */
async function makeSlot(root: string, name: string): Promise<Slot> {
  const dir = join(root, name);
  const template = join(root, `${name}-template`);
  const first = await workspace(dir, true);
  await first.stop();
  cpSync(dir, template, { recursive: true, verbatimSymlinks: true });
  return { dir, template };
}

/** A worker's daemons. Booting a workspace from nothing is most of a test's
 *  setup and the same every time, so each slot is made once per worker;
 *  a test gets a fresh copy of a slot's template and a daemon over it. While
 *  one test runs on one slot, the other slot's daemon boots for the next
 *  test, so a test never waits for a boot but the worker's first. */
class Daemons {
  private turn = 0;
  private next: Promise<Workspace> | null = null;

  constructor(private readonly slots: [Slot, Slot]) {}

  private boot(slot: Slot): Promise<Workspace> {
    rmSync(slot.dir, { recursive: true, force: true });
    cpSync(slot.template, slot.dir, { recursive: true, verbatimSymlinks: true });
    const booting = workspace(slot.dir);
    // A boot that fails is the next test's failure, not an unhandled one.
    booting.catch(() => {});
    return booting;
  }

  /** The next test's daemon, and the one after it set booting. The slot it
   *  boots in is the one the last test used, which that test stopped. */
  async take(): Promise<Workspace> {
    const current = this.next ?? this.boot(this.slots[this.turn]!);
    this.turn = 1 - this.turn;
    this.next = null;
    const daemon = await current;
    this.next = this.boot(this.slots[this.turn]!);
    return daemon;
  }

  /** Stop the daemon booted for a test that never came. */
  async close(): Promise<void> {
    const left = this.next;
    this.next = null;
    if (left) await left.then((d) => d.stop(), () => {});
  }
}

export const test = base.extend<{ daemon: Workspace }, { daemons: Daemons }>({
  daemons: [
    async ({}, use) => {
      const root = mkdtempSync(join(tmpdir(), "oxplow-e2e-w"));
      try {
        const slots = await Promise.all([makeSlot(root, "a"), makeSlot(root, "b")]);
        const daemons = new Daemons(slots);
        try {
          await use(daemons);
        } finally {
          await daemons.close();
        }
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    },
    { scope: "worker", timeout: 180_000 },
  ],
  // A daemon of the spec's own, over its own copy of a template, stopped
  // after it: no spec meets what another left in the project, the settings
  // or the daemon's tabs and streams. A spec on `page` or `daemon` gets it.
  daemon: [
    async ({ daemons }, use) => {
      const daemon = await daemons.take();
      try {
        await use(daemon);
      } finally {
        await daemon.stop();
      }
    },
    { timeout: 180_000 },
  ],
  // The transport reads its daemon from localStorage at load.
  storageState: async ({ daemon, baseURL }, use) => {
    await use(connectedTo(baseURL!, daemon.base, daemon.token));
  },
  page: async ({ page }, use) => {
    const check = guard(page);
    await use(page);
    check();
  },
});

export { expect };
