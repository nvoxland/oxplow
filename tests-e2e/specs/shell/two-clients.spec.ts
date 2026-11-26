// One person on two clients of the same projects: what each client keeps of
// its own, and what they share. A test that fails on today's code is marked
// `test.fail()`: the mark comes off in the change that makes it pass.
import { ipc, run, settle, startDaemon } from "../../support/daemon.js";
import { client, connectContext, expect, test } from "../../support/fixtures.js";
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
    // The command opens a page on the window used last: this one, now.
    await load(b.page);
    await b.page.mouse.click(5, 5);
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

/// A client watching a terminal another client sized draws that grid, scaled
/// to its pane, and Fit to this device takes the size for it.
test("a watching client draws the owner's grid, and Fit takes it", async ({ page, browser, baseURL, daemon }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("echo owner");
  await page.keyboard.press("Enter");

  const b = await client(browser, baseURL!, daemon, { width: 700, height: 600 });
  try {
    // The command opens a page on the window used last: this one, now.
    await load(b.page);
    await b.page.mouse.click(5, 5);
    await open(b.page, daemon, "oxplow:oxplow/page?page=terminal");
    const sized = b.page.getByTestId("terminal-sized-elsewhere");
    await expect(sized).toBeVisible();
    // The owner's grid, drawn in a smaller font to fit the narrow pane. A
    // CSS transform would put the pointer in the wrong cell, so there is none.
    await expect(b.page.locator(".xterm")).toHaveCSS("transform", "none");
    const fontPx = async () => parseFloat(await b.page.locator(".xterm-rows").evaluate((el) => getComputedStyle(el).fontSize));
    expect(await fontPx()).toBeLessThan(13);
    // The grid fits the pane: not wider than its mount.
    const [screen, mount] = await Promise.all([
      b.page.locator(".xterm-screen").boundingBox(),
      b.page.getByTestId("terminal-mount").boundingBox(),
    ]);
    expect(screen!.width).toBeLessThanOrEqual(mount!.width + 1);

    await b.page.getByTestId("terminal-fit").click();
    await expect(sized).toBeHidden();
    // The shell now has the narrow client's size, and the wide one draws it.
    await expect(page.getByTestId("terminal-sized-elsewhere")).toBeVisible();
    await b.page.locator(".xterm").click();
    await b.page.keyboard.type("echo fit-$((6*7))=$(stty size | tr ' ' x)");
    await b.page.keyboard.press("Enter");
    await expect(page.locator(".xterm-rows")).toContainText(/fit-42=\d+x\d+/);
  } finally {
    await b.context.close();
  }
});

/// The client watching a terminal takes its size by typing, with no Fit:
/// its keys are what takes it, and the daemon tells who owns it. A reply
/// xterm sends to a program's query takes nothing (the watching client's
/// replay holds one).
test("a watching client that types takes the terminal's size", async ({ page, browser, baseURL, daemon }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  // A device-attributes query in the terminal's replay: the watching client's
  // xterm answers it when it opens, which must not take the size.
  await page.keyboard.type("printf '\\033[c'; echo owner-$((6*7))");
  await page.keyboard.press("Enter");
  await expect(page.locator(".xterm-rows")).toContainText("owner-42");

  const b = await client(browser, baseURL!, daemon, { width: 700, height: 600 });
  try {
    await load(b.page);
    await b.page.mouse.click(5, 5);
    await open(b.page, daemon, "oxplow:oxplow/page?page=terminal");
    const sized = b.page.getByTestId("terminal-sized-elsewhere");
    await expect(sized).toBeVisible();
    // Opening and its replies took nothing: once the replay is drawn (the
    // query in it answered), the wide client still owns it.
    await expect(b.page.locator(".xterm-rows")).toContainText("owner-42");
    await expect(sized).toBeVisible();
    await expect(page.getByTestId("terminal-sized-elsewhere")).toBeHidden();

    await b.page.locator(".xterm").click();
    // The replies to the query went to the shell's line as text: clear it.
    await b.page.keyboard.press("Control+u");
    await b.page.keyboard.type("echo typed-$((6*7))=$(stty size | tr ' ' x)");
    await b.page.keyboard.press("Enter");
    // Its keys took the size: it owns the terminal, the wide client watches.
    await expect(sized).toBeHidden();
    await expect(page.getByTestId("terminal-sized-elsewhere")).toBeVisible();
    await expect(b.page.locator(".xterm-rows")).toContainText(/typed-42=\d+x\d+/);
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

    const context = await browser.newContext({
      baseURL,
      storageState: {
        cookies: [],
        origins: [{ origin: new URL(baseURL!).origin, localStorage: [{ name: DEVICE_KEY, value: device! }] }],
      },
    });
    await connectContext(context, second.base, second.token);
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
