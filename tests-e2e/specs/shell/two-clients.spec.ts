// One person on two clients of the same projects: what each client keeps of
// its own, and what they share. A test that fails on today's code is marked
// `test.fail()`: the mark comes off in the change that makes it pass
// (the shared terminal's size rule).
import { ipc, run, settle, startDaemon } from "../../support/daemon.js";
import { client, connectedTo, expect, test } from "../../support/fixtures.js";
import { load, open, reload } from "../../support/ui.js";

/** Where a browser keeps its device id (`device.ts`). */
const DEVICE_KEY = "oxplow.device";

/// The thread a client shows is that client's own: another client picking a
/// thread doesn't move it, even across a reload.
test("a thread picked on one client is not the other client's thread", async ({ page, browser, baseURL, daemon }) => {
  const other = await run<{ result: { id: string } }>(daemon, "oxplow.thread.create", {
    stream: daemon.stream,
    title: "Second client's thread",
  });
  await load(page);
  const first = await page.getByTestId("title-bar-thread").textContent();

  const b = await client(browser, baseURL!, daemon);
  try {
    await load(b.page);
    await b.page.getByTestId(`navigator-strip-thread-${other.result.id}`).click();
    await expect(b.page.getByTestId("title-bar-thread")).toHaveText("Second client's thread");

    await reload(page);
    await expect(page.getByTestId("title-bar-thread")).toHaveText(first!);
  } finally {
    await b.context.close();
  }
});

/// The stream a client shows is that client's own: another client switching
/// stream doesn't move it, even across a reload.
test("the stream a client shows is that client's own", async ({ page, browser, baseURL, daemon }) => {
  const made = await run<{ result: { id: string } }>(daemon, "oxplow.stream.create", {
    title: "Second client's stream",
    source: "new",
    new_branch: "second-client",
    start_point: "HEAD",
  });
  await load(page);
  const first = await page.getByTestId("title-bar-stream").textContent();

  const b = await client(browser, baseURL!, daemon);
  try {
    await load(b.page);
    await b.page.getByTestId(`navigator-strip-stream-${made.result.id}`).click();
    await expect(b.page.getByTestId("title-bar-stream")).toContainText("Second client's stream");

    await reload(page);
    await expect(page.getByTestId("title-bar-stream")).toHaveText(first!);
  } finally {
    await b.context.close();
  }
});

/// A shared terminal keeps the size of the client typing into it: another
/// client opening it doesn't shrink it under the first one's feet.
test("a terminal another client opens keeps the size of the client typing", async ({ page, browser, baseURL, daemon }) => {
  test.fail(true, "every attach resizes the terminal to the attaching pane");
  await page.setViewportSize({ width: 1400, height: 900 });
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  // The shell's own idea of its size, before and after: `stty size` is
  // "rows cols". The 6*7 makes the output line, not the typed one, match.
  await page.keyboard.type("echo before-$((6*7))=$(stty size | tr ' ' x)");
  await page.keyboard.press("Enter");
  const rows = page.locator(".xterm-rows");
  await expect(rows).toContainText(/before-42=\d+x\d+/);
  const before = /before-42=(\d+x\d+)/.exec((await rows.textContent()) ?? "")![1];

  // A narrow client opens the same terminal.
  const b = await client(browser, baseURL!, daemon, { width: 700, height: 600 });
  try {
    await open(b.page, daemon, "oxplow:oxplow/page?page=terminal");
    await expect(b.page.locator(".xterm")).toBeVisible();

    await terminal.click();
    await page.keyboard.type("echo after-$((6*7))=$(stty size | tr ' ' x)");
    await page.keyboard.press("Enter");
    await expect(rows).toContainText(/after-42=\d+x\d+/);
    const after = /after-42=(\d+x\d+)/.exec((await rows.textContent()) ?? "")![1];
    expect(after).toBe(before);
  } finally {
    await b.context.close();
  }
});

/// Two projects' windows share a browser origin (every Mac window is
/// `tauri://localhost`), and their threads' ids can be the same. A project's
/// window shows its own tabs, never another project's.
test("a project's window never shows another project's tabs", async ({ page, browser, baseURL, daemon }) => {
  test.setTimeout(180_000);
  const second = await startDaemon();
  try {
    await settle(second);
    const [stream] = await ipc<{ id: string }[]>(second, "list_streams");
    const [thread] = await ipc<{ id: string }[]>(second, "list_threads", { streamId: stream!.id });
    // The two projects' ids meet only when they are the same, as two fresh
    // projects' are.
    expect(thread!.id, "the two fresh projects' threads have the same id").toBe(daemon.thread);

    await open(page, daemon, "oxplow:oxplow/page?page=settings");
    await expect(page.getByTestId("center-tab-oxplow:oxplow/page?page=settings")).toBeVisible();
    // The second project's window is the same device (the same browser).
    const device = await page.evaluate((key) => localStorage.getItem(key), DEVICE_KEY);
    expect(device).toMatch(/^dev-/);

    const state = connectedTo(baseURL!, second.base, second.token);
    state.origins[0]!.localStorage.push({ name: DEVICE_KEY, value: device! });
    const context = await browser.newContext({ storageState: state });
    try {
      const other = await context.newPage();
      await load(other);
      await expect(other.getByTestId("title-bar")).toBeVisible();
      await expect(other.getByTestId("center-tab-oxplow:oxplow/page?page=settings")).toHaveCount(0);
    } finally {
      await context.close();
    }
  } finally {
    await second.stop();
  }
});

const SHOWN = `title: Sent Quokka
loaders:
  count:
    sql: SELECT count(*) AS n FROM v_oxplow_task
view: { id: count, viz: number, from: count }
`;

/// An agent's open names the device it is for: the second client gets the
/// tab, the first never does. The agent's tool runs the bus command
/// `oxplow.page.show` (a spec runs it as the person, which is the same call
/// with no device of its own).
test("a page shown on a named device opens on that device only", async ({ page, browser, baseURL, daemon }) => {
  await load(page);
  const b = await client(browser, baseURL!, daemon);
  try {
    await load(b.page);
    const device = await b.page.evaluate((key) => localStorage.getItem(key), DEVICE_KEY);
    expect(device).toMatch(/^dev-/);
    const shown = await run<{ result: { uri: string; opened: boolean; why?: string } }>(daemon, "oxplow.page.show", {
      spec: SHOWN,
      client: device,
    });
    expect(shown.result.opened, shown.result.why).toBe(true);
    await expect(b.page.getByTestId(`center-tab-${shown.result.uri}`)).toBeVisible();
    await expect(page.getByTestId(`center-tab-${shown.result.uri}`)).toBeHidden();
  } finally {
    await b.context.close();
  }
});
