import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Terminal page is a plain shell in the stream's worktree.
test("a terminal opens a shell in the project", async ({ page }) => {
  // The terminal's web font arrives only once its pane is on screen, as on
  // a first load. The terminal measures its rows once, when it opens, so it
  // must open in that font, not in the fallback the font then replaces:
  // rows measured in the fallback overflow the pane at its next fit
  // (tsk1042, under the Answers strip).
  let releaseFont = () => {};
  const fontHeld = new Promise<void>((resolve) => (releaseFont = resolve));
  await page.route("**/fonts/JetBrainsMonoVariable.woff2", async (route) => {
    await fontHeld;
    await route.continue();
  });
  await page.addInitScript(() => {
    new MutationObserver((_, observer) => {
      const term = document.querySelector(".xterm");
      if (!term) return;
      // The face's own status: `document.fonts.check` answers true while
      // it is still loading.
      (window as unknown as { openedInFont: boolean }).openedInFont = [...document.fonts]
        .filter((f) => f.family === "JetBrains Mono" && f.style === "normal")
        .every((f) => f.status === "loaded");
      observer.disconnect();
    }).observe(document, { childList: true, subtree: true });
  });
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  await expect(page.getByTestId("terminal-mount")).toBeVisible();
  releaseFont();
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { openedInFont: boolean }).openedInFont)).toBe(true);
  await terminal.click();
  // The output, not the typed line: only the shell computes 42.
  await page.keyboard.type("echo e2e-$((6*7))");
  await page.keyboard.press("Enter");
  await expect(page.locator(".xterm-rows")).toContainText("e2e-42");
  // It fits its mount: the last row isn't clipped below it.
  const fit = await terminal.evaluate((el) => ({
    screen: el.getBoundingClientRect().height,
    mount: (el.parentElement as HTMLElement).getBoundingClientRect().height,
  }));
  expect(fit.screen).toBeLessThanOrEqual(fit.mount + 1);
});

// Keys typed the moment the terminal shows — before its shell is up —
// arrive, all of them, in the order typed: the daemon holds them for the
// PTY, and one socket keeps them in order.
test("a burst typed as the terminal appears arrives whole and in order", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  const run = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(3);
  await page.keyboard.type(`echo burst-${run}-$((6*7))`);
  await page.keyboard.press("Enter");
  // The output, not the typed line: only the shell computes 42.
  await expect(page.locator(".xterm-rows")).toContainText(`burst-${run}-42`);
});

// tsk1026: a session whose process exited says so, takes no keys, and Start
// Again opens a fresh one in the same pane.
test("an exited shell says so, and Start Again opens a new one", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("exit");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("terminal-ended")).toContainText("Session ended");
  await page.getByTestId("terminal-start-again").click();
  await expect(page.getByTestId("terminal-ended")).toHaveCount(0);
  await expect(page.locator(".xterm-rows")).toContainText("started again");
  await terminal.click();
  await page.keyboard.type("echo again-$((6*7))");
  await page.keyboard.press("Enter");
  await expect(page.locator(".xterm-rows")).toContainText("again-42");
});

// Terminals are rows: opening the page gives the stream its first, closing
// it ends the shell and deletes the row, and the page then offers New.
test("a terminal is a row: closed with its shell, and New opens another", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const rows = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT id, uri, title FROM v_terminal" })).rows;
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  await expect(page.locator(".xterm")).toBeVisible();
  await until("the first terminal's row", 15_000, async () => (await rows()).length === 1);
  const [id, uri, title] = (await rows())[0] as [number, string, string];
  expect(title).toBe("Terminal 1");
  expect(uri).toBe(`oxplow:oxplow/terminal?id=trm${id}`);

  await run(daemon, "oxplow.terminal.close", { terminal: `trm${id}` });
  await until("the row to go", 15_000, async () => (await rows()).length === 0);
  await expect(page.getByTestId("terminal-new")).toBeVisible();
  await page.getByTestId("terminal-new").click();
  await until("a new row", 15_000, async () => (await rows()).length === 1);
  await expect(page.locator(".xterm")).toBeVisible();
});

// Several terminals: the strip picks one, its menu renames one through
// the page's action and closes one after asking.
test("a second terminal from the strip is picked, renamed and closed", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const rows = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT id, title FROM v_terminal ORDER BY position, id" })).rows;
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  await expect(page.locator(".xterm").first()).toBeVisible();
  await until("the first terminal's row", 15_000, async () => (await rows()).length === 1);
  await run(daemon, "oxplow.terminal.open", { stream: "str1" });
  await until("a second row", 15_000, async () => (await rows()).length === 2);
  const [secondId] = (await rows())[1] as [number, string];
  const second = `oxplow:oxplow/terminal?id=trm${secondId}`;
  await expect(page.getByTestId("terminal-tab-strip")).toBeVisible();
  await page.getByTestId(`terminal-tab-${second}`).click();
  await expect(page.locator(`[data-testid="terminal-view"][data-uri="${second}"] .xterm`)).toBeVisible();

  await page.getByTestId("terminal-tab-expand").click();
  await page.getByTestId(`terminal-tab-row-${second}`).click({ button: "right" });
  await page.getByTestId("menu-item-terminal.rename").click();
  const name = page.getByTestId(`terminal-tab-rename-input-${second}`);
  await name.fill("Builds");
  await name.press("Enter");
  await until("the rename", 15_000, async () => ((await rows())[1] as [number, string])[1] === "Builds");

  // The panel stays open after a rename.
  await page.getByTestId(`terminal-tab-row-${second}`).click({ button: "right" });
  await page.getByTestId("menu-item-terminal.close").click();
  await page.getByTestId("viz-action-confirm-run").click();
  await until("the close", 15_000, async () => (await rows()).length === 1);
  await expect(page.getByTestId("terminal-tab-strip")).toHaveCount(0);
});
