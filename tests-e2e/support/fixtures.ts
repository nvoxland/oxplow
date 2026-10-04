// The suite's fixtures: a daemon per worker, a page that talks to it, and a
// guard that fails any spec whose page threw.
import { test as base, expect } from "@playwright/test";

import { ipc, run, startDaemon, type Daemon } from "./daemon.js";

type Stream = { id: string };
type Thread = { id: string; agent: string };

/** Select an ACP thread on the fake agent before any page opens: the boot
 *  thread is a terminal agent's, and the suite never starts a real agent CLI. */
async function selectFakeAgentThread(daemon: Daemon): Promise<void> {
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
}

export const test = base.extend<{ pageErrors: string[] }, { daemon: Daemon }>({
  daemon: [
    async ({}, use) => {
      const daemon = await startDaemon();
      await selectFakeAgentThread(daemon);
      await use(daemon);
      await daemon.stop();
    },
    { scope: "worker", timeout: 180_000 },
  ],
  // The transport reads its daemon from localStorage at load.
  storageState: async ({ daemon, baseURL }, use) => {
    await use({
      cookies: [],
      origins: [
        {
          origin: new URL(baseURL!).origin,
          localStorage: [
            { name: "oxplow.remoteBase", value: daemon.base },
            { name: "oxplow.remoteToken", value: daemon.token },
          ],
        },
      ],
    });
  },
  pageErrors: [
    async ({ page }, use) => {
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(String(e)));
      await use(errors);
      expect(errors, "the page threw").toEqual([]);
    },
    { auto: true },
  ],
});

export { expect };
