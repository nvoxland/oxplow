// The suite's fixtures: a daemon per worker, a page that talks to it, and a
// guard that fails any spec whose page threw. A spec on `fresh` alone never
// boots the worker's daemon: only `page` (through `storageState`) asks for it.
import { test as base, expect, type Page } from "@playwright/test";

import { ipc, run, settle, startDaemon, until, type Daemon } from "./daemon.js";

type Stream = { id: string };
type Thread = { id: string; agent: string };

/** Select an ACP thread on the fake agent before any page opens: the boot
 *  thread is a terminal agent's, and the suite never starts a real agent CLI. */
async function selectFakeAgentThread(daemon: Daemon): Promise<{ stream: string; thread: string }> {
  const [stream] = await ipc<Stream[]>(daemon, "list_streams");
  if (!stream) throw new Error("the daemon has no stream");
  const threads = await ipc<Thread[]>(daemon, "list_threads", { streamId: stream.id });
  let acp = threads.find((t) => t.agent === "acp");
  if (!acp) {
    await run(daemon, "thread.create", { stream: `stream:${stream.id}`, title: "Fake agent", agent: "acp", acp_agent: "fake" });
    acp = (await ipc<Thread[]>(daemon, "list_threads", { streamId: stream.id })).find((t) => t.agent === "acp");
  }
  if (!acp) throw new Error("no ACP thread");
  await ipc(daemon, "select_thread", { req: { streamId: stream.id, threadId: acp.id } });
  return { stream: stream.id, thread: acp.id };
}

/** A worker's daemon, and the stream and thread its pages open on. */
export type Workspace = Daemon & { stream: string; thread: string };

/** A daemon booted, settled and pointed at the fake agent's thread; one
 *  that fails on the way is stopped, its project removed. */
async function workspace(): Promise<Workspace> {
  const daemon = await startDaemon();
  try {
    await settle(daemon);
    // Extensions' models compile in the background after boot: the test
    // extension's is published once they are.
    await until("the extensions' models to publish", 60_000, async () => {
      const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT 1 FROM v_model WHERE view = 'v_e2e_item'",
      });
      return out.rows.length > 0;
    });
    return { ...daemon, ...(await selectFakeAgentThread(daemon)) };
  } catch (e) {
    await daemon.stop();
    throw e;
  }
}

/** Collect what `page` throws; check it once the spec is done. */
function guard(page: Page): () => void {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  return () => expect(errors, "the page threw").toEqual([]);
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

export const test = base.extend<{ fresh: { daemon: Workspace; page: Page } }, { daemon: Workspace }>({
  daemon: [
    async ({}, use) => {
      const daemon = await workspace();
      try {
        await use(daemon);
      } finally {
        await daemon.stop();
      }
    },
    { scope: "worker", timeout: 180_000 },
  ],
  // A daemon of the spec's own, for one whose state no other spec may
  // touch first (nothing approved yet, an empty project), and a page on it.
  fresh: [
    async ({ browser, baseURL }, use) => {
      const daemon = await workspace();
      try {
        const context = await browser.newContext({ storageState: connectedTo(baseURL!, daemon.base, daemon.token) });
        try {
          const page = await context.newPage();
          const check = guard(page);
          await use({ daemon, page });
          check();
        } finally {
          await context.close();
        }
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
