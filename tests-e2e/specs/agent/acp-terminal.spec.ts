import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The suite's threads run the fake ACP agent, which does what each
// `fake:<step>` line of a prompt says.

test("the fake agent's reply streams into the thread's transcript", async ({ page }) => {
  // An ACP agent is a project program; the workspace approved it.
  await page.goto("/");
  const input = page.getByTestId("acp-prompt-input");
  await input.fill("fake:say Hello from the fake agent");
  await input.press("Enter");
  // The agent's own message, not the prompt that asked for it.
  const said = page.getByTestId("acp-transcript").locator('[data-kind="agent"]');
  await expect(said).toContainText("Hello from the fake agent");
});

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
