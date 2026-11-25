// What the window does today, pinned before App.tsx is split into hooks:
// stream and thread switching, the tab cap, resync after a dropped event
// socket, and who a tab open reaches. Each test asserts current behavior.
import { ipc, run, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { load, open, shown } from "../../support/ui.js";
import type { Page } from "@playwright/test";

const tab = (page: Page, uri: string) => page.getByTestId(`center-tab-${uri}`);
const home = "oxplow:oxplow/page?page=e2e/home";
const second = "oxplow:oxplow/page?page=e2e/second";
const overview = "oxplow:oxplow/page?page=e2e/overview";

async function newThread(daemon: Daemon, title: string, stream = daemon.stream) {
  const made = await run<{ result: { id: string } }>(daemon, "oxplow.thread.create", { stream, title });
  return made.result.id;
}

test("switching stream shows that stream's thread and tabs, and back restores the first's", async ({ page, daemon }) => {
  await load(page);
  await open(page, daemon, home);
  const firstStream = (await page.getByTestId("title-bar-stream").textContent())!;
  const firstThread = (await page.getByTestId("title-bar-thread").textContent())!;

  const made = await run<{ result: { id: string } }>(daemon, "oxplow.stream.create", {
    title: "Other stream",
    source: "new",
    new_branch: "other-stream",
    start_point: "HEAD",
  });
  const [thread] = await ipc<{ id: string; title: string }[]>(daemon, "list_threads", { streamId: made.result.id });

  await page.getByTestId(`navigator-strip-stream-${made.result.id}`).click();
  // A click on the strip opens the navigator's overlay, which covers the strip.
  await expect(page.getByTestId("navigator-overlay")).toBeVisible();
  await expect(page.getByTestId("title-bar-stream")).toContainText("Other stream");
  await expect(page.getByTestId("title-bar-thread")).toHaveText(thread!.title);
  // The first thread's tab is not the second stream's.
  await expect(tab(page, home)).not.toBeVisible();
  await expect(shown(page, home)).toHaveCount(0);

  await page.getByTestId(`navigator-stream-row-${daemon.stream}`).click();
  await expect(page.getByTestId("title-bar-stream")).toHaveText(firstStream);
  await expect(page.getByTestId("title-bar-thread")).toHaveText(firstThread);
  await expect(tab(page, home)).toBeVisible();
  await expect(shown(page, home)).toHaveCount(1);
});

test("selecting a thread shows its own tabs, and the first thread returns with its active tab", async ({ page, daemon }) => {
  await load(page);
  await open(page, daemon, home);
  await open(page, daemon, second, { new_tab: true });
  const other = await newThread(daemon, "Second thread");

  await page.getByTestId(`navigator-strip-thread-${other}`).click();
  await expect(page.getByTestId("title-bar-thread")).toHaveText("Second thread");
  await expect(tab(page, home)).not.toBeVisible();
  await expect(tab(page, second)).not.toBeVisible();
  await open(page, daemon, overview, { new_tab: true });
  await expect(tab(page, overview)).toBeVisible();

  await page.getByTestId(`navigator-strip-thread-${daemon.thread}`).click();
  await expect(tab(page, home)).toBeVisible();
  await expect(tab(page, second)).toBeVisible();
  await expect(tab(page, overview)).not.toBeVisible();
  // `second` was the active tab when the thread was left.
  await expect(shown(page, second)).toHaveCount(1);
});

// MAX_PAGE_TABS is 15 and pinned tabs (an agent session's) are not counted:
// the 16th page closes the least recently used one.
test("past the tab cap the least recently used page tab closes and the session tab stays", async ({ page, daemon }) => {
  test.setTimeout(120_000);
  const cap = 15;
  // Wide enough that all the tabs fit the strip, which draws only those.
  await page.setViewportSize({ width: 3200, height: 900 });
  await load(page);
  await page.locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const session = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(session).toBeVisible();

  // Distinct pages: one page with a param of its own each (a file's tab
  // opens only once its buffer is read, which drops an open now and then).
  const uris = Array.from({ length: cap + 1 }, (_, i) => `oxplow:oxplow/page?page=e2e/item&uri=cap-${String(i).padStart(2, "0")}`);
  for (const u of uris.slice(0, cap)) await open(page, daemon, u, { new_tab: true });
  // At the cap nothing has closed yet.
  await expect(tab(page, uris[0]!)).toBeVisible();

  await open(page, daemon, uris[cap]!, { new_tab: true });
  await expect(tab(page, uris[0]!)).not.toBeVisible();
  for (const u of uris.slice(1)) await expect(tab(page, u)).toBeVisible();
  await expect(session).toBeVisible();
});

test("after the event socket drops and returns, the window shows what changed meanwhile", async ({ page, daemon }) => {
  // Every socket the page opens is kept, and while `hold` is set a new one
  // is pointed at a dead port, so the change is made with the window offline.
  await page.addInitScript(() => {
    const w = window as unknown as { __sockets: WebSocket[]; __hold: boolean };
    w.__sockets = [];
    w.__hold = false;
    const Real = window.WebSocket;
    window.WebSocket = class extends Real {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(w.__hold ? "ws://127.0.0.1:1/" : url, protocols);
        w.__sockets.push(this);
      }
    } as typeof WebSocket;
  });
  await load(page);
  const before = (await page.getByTestId("title-bar-thread").textContent())!;

  await page.evaluate(() => {
    const w = window as unknown as { __sockets: WebSocket[]; __hold: boolean };
    w.__hold = true;
    for (const s of w.__sockets) s.close();
  });
  await run(daemon, "oxplow.thread.rename", { thread: daemon.thread, title: `${before} renamed` });
  await page.evaluate(() => ((window as unknown as { __hold: boolean }).__hold = false));
  await expect(page.getByTestId("title-bar-thread")).toHaveText(`${before} renamed`);
});

// An agent's tab open carries its thread (`ClientCall.thread_id`) and lands
// in that thread's tabs, shown or not. A spec cannot make that call: the
// agent's identity is a session bearer only a launched agent holds. The
// person's `oxplow.tab.open` is the nearest: it has no thread, so it lands
// in the thread the window shows, and the other thread is untouched.
test("a person's tab open lands in the shown thread, not in another thread", async ({ page, daemon }) => {
  await load(page);
  const other = await newThread(daemon, "Other thread");
  await run(daemon, "oxplow.tab.open", { uri: second });
  await expect(tab(page, second)).toBeVisible();
  const title = (await page.getByTestId("title-bar-thread").textContent())!;

  await page.getByTestId(`navigator-strip-thread-${other}`).click();
  await expect(page.getByTestId("title-bar-thread")).toHaveText("Other thread");
  await expect(tab(page, second)).not.toBeVisible();
  await page.getByTestId(`navigator-strip-thread-${daemon.thread}`).click();
  await expect(page.getByTestId("title-bar-thread")).toHaveText(title);
  await expect(tab(page, second)).toBeVisible();
});
