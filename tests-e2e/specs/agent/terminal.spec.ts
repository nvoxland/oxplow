import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// The Terminal page is a plain shell in the stream's worktree.
test("a terminal opens a shell in the project", async ({ page, daemon }) => {
  // The terminal's web font arrives only once its pane is on screen, as on
  // a first load. The terminal measures its rows once, when it opens, so it
  // must open in that font, not in the fallback the font then replaces:
  // rows measured in the fallback overflow the pane at its next fit
  // (under the Answers strip).
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
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
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
test("a burst typed as the terminal appears arrives whole and in order", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  const run = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(3);
  await page.keyboard.type(`echo burst-${run}-$((6*7))`);
  await page.keyboard.press("Enter");
  // The output, not the typed line: only the shell computes 42.
  await expect(page.locator(".xterm-rows")).toContainText(`burst-${run}-42`);
});

// a session whose process exited says so, takes no keys, and Start
// Again opens a fresh one in the same pane.
test("an exited shell says so, and Start Again opens a new one", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("exit");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("terminal-ended")).toContainText("Session ended");
  await page.getByTestId("terminal-start-again").click();
  await expect(shown(page).getByTestId("terminal-ended")).toHaveCount(0);
  await expect(page.locator(".xterm-rows")).toContainText("started again");
  await terminal.click();
  await page.keyboard.type("echo again-$((6*7))");
  await page.keyboard.press("Enter");
  await expect(page.locator(".xterm-rows")).toContainText("again-42");
});

// Terminals are rows: opening the page gives the stream its first, closing
// it ends the shell and deletes the row, and the page then offers New.
test("a terminal is a row: closed with its shell, and New opens another", async ({ page, daemon }) => {
  const rows = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT id, uri, title FROM v_terminal" })).rows;
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
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
test("a second terminal from the strip is picked, renamed and closed", async ({ page, daemon }) => {
  const rows = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT id, title FROM v_terminal ORDER BY position, id" })).rows;
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
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

  // The panel stays open after a rename. Close names the row as the page last
  // drew it; the daemon refuses a row the page has since changed, so wait for
  // the page to draw the new title.
  await expect(page.getByTestId(`terminal-tab-row-${second}`)).toContainText("Builds");
  await page.getByTestId(`terminal-tab-row-${second}`).click({ button: "right" });
  await page.getByTestId("menu-item-terminal.close").click();
  await page.getByTestId("viz-action-confirm-run").click();
  await until("the close", 15_000, async () => (await rows()).length === 1);
  await expect(shown(page).getByTestId("terminal-tab-strip")).toHaveCount(0);
});

// A selection in the terminal goes in the basket from its right-click
// menu, never a floating button: the terminal's own selection (not what
// the browser selected in xterm's rows) and the lines around it, which the
// scrollback can't give back later.
test("a terminal selection goes in the basket with the lines around it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("printf 'a1\\na2\\na3\\na4\\n%s\\nb1\\nb2\\nb3\\nb4\\n' select-$((6*7))-here");
  await page.keyboard.press("Enter");
  // A row is the output line (the typed command holds `$((6*7))`, not 42).
  const row = page.locator(".xterm-rows > div").filter({ hasText: /^\s*select-42-here\s*$/ });
  await expect(row).toBeVisible();
  // xterm takes the mouse on a layer over its rows: press where the row is.
  const box = (await row.boundingBox())!;
  await page.mouse.dblclick(box.x + 10, box.y + box.height / 2);
  await expect(shown(page).getByTestId("selection-context-button")).toHaveCount(0);
  await page.mouse.click(box.x + 10, box.y + box.height / 2, { button: "right" });
  await page.getByTestId("menu-item-add-to-context").click();
  await page.getByTestId("context-note-add").click();
  await expect(page.getByTestId("dock-context-badge")).toHaveText("1");
  const item = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT text, around FROM v_context_item" })).rows;
  await until("the item", 15_000, async () => (await item()).length === 1);
  const [text, around] = (await item())[0] as [string, string];
  expect(text).toBe("select-42-here");
  // Three rows either side, exactly.
  expect(around).toBe("a2\na3\na4\nselect-42-here\nb1\nb2\nb3");
});

// A right-click keeps the selection: the menu's Add Selection to
// Context adds what was dragged across, not the word under the pointer
// (xterm's `rightClickSelectsWord`, on by default on macOS, is off).
test("a right-click adds the whole dragged selection, not the word under it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("printf '%s\\n' drag-$((6*7))-across-these-words");
  await page.keyboard.press("Enter");
  const row = page.locator(".xterm-rows > div").filter({ hasText: /^\s*drag-42-across-these-words\s*$/ });
  await expect(row).toBeVisible();
  const box = (await row.boundingBox())!;
  const text = await row.evaluate((el) => (el.textContent ?? "").trimEnd().length);
  const charWidth = await row.evaluate((el) => {
    const r = document.createRange();
    r.selectNodeContents(el);
    return r.getBoundingClientRect().width / Math.max(1, (el.textContent ?? "").length);
  });
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + 1, y);
  await page.mouse.down();
  await page.mouse.move(box.x + charWidth * text - 1, y, { steps: 5 });
  await page.mouse.up();
  // Over a word of another row: xterm would select that word in its place
  // when it judges the press outside the selection (as the app's zoom made
  // it judge a press inside one).
  await page.mouse.click(box.x + charWidth * 3, y - box.height, { button: "right" });
  await page.getByTestId("menu-item-add-to-context").click();
  await page.getByTestId("context-note-add").click();
  const item = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT text FROM v_context_item" })).rows;
  await until("the item", 15_000, async () => (await item()).length === 1);
  expect(((await item())[0] as [string])[0]).toBe("drag-42-across-these-words");
});

// A program that has the mouse (Claude Code turns on mouse reporting, every
// motion, SGR-encoded) gets a plain drag, so the terminal selects nothing
// and its menu says how to: Option+drag on macOS (Shift elsewhere) makes
// the terminal's own selection. Moving the mouse and right-clicking after
// the modifier is let go keep it (xterm would report them to the program,
// which clears a selection), and the menu adds it.
test("with the mouse a program's, Option+drag selects and the menu adds it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  await page.keyboard.type("printf '%s\\n\\033[?1003h\\033[?1006h' held-$((6*7))-by-a-program");
  await page.keyboard.press("Enter");
  const row = page.locator(".xterm-rows > div").filter({ hasText: /^\s*held-42-by-a-program\s*$/ });
  await expect(row).toBeVisible();
  const box = (await row.boundingBox())!;
  const charWidth = await row.evaluate((el) => {
    const r = document.createRange();
    r.selectNodeContents(el);
    return r.getBoundingClientRect().width / Math.max(1, (el.textContent ?? "").length);
  });
  const y = box.y + box.height / 2;
  const drag = async () => {
    await page.mouse.move(box.x + 1, y);
    await page.mouse.down();
    await page.mouse.move(box.x + charWidth * "held-42-by-a-program".length - 1, y, { steps: 5 });
    await page.mouse.up();
  };
  await drag();
  await page.mouse.click(box.x + 10, y, { button: "right" });
  await expect(page.getByTestId("menu-item-select-hint")).toBeVisible();
  await page.keyboard.press("Escape");
  const modifier = process.platform === "darwin" ? "Alt" : "Shift";
  await page.keyboard.down(modifier);
  await drag();
  await page.keyboard.up(modifier);
  await page.mouse.move(box.x + 40, y + box.height * 2, { steps: 5 });
  await page.mouse.click(box.x + 10, y, { button: "right" });
  const add = page.getByTestId("menu-item-add-to-context");
  await expect(add).toHaveText(/Add Selection to Context/);
  await add.click();
  await page.getByTestId("context-note-add").click();
  const item = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT text FROM v_context_item" })).rows;
  await until("the item", 15_000, async () => (await item()).length === 1);
  expect(((await item())[0] as [string])[0]).toBe("held-42-by-a-program");
});
