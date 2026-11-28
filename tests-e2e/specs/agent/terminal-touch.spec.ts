import { client, expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// On a touch screen the terminal has a bar of the keys an on-screen keyboard
// lacks, and Compose: a real text field. An agent's first question needs
// the arrows, and an interrupt needs Ctrl+C or Esc.
test("the touch bar sends the keys a touch keyboard lacks", async ({ browser, baseURL, daemon }) => {
  const tablet = await client(browser, baseURL!, daemon, { width: 1180, height: 820 }, { hasTouch: true });
  const page = tablet.page;
  try {
    await open(page, daemon, "oxplow:oxplow/page?page=terminal");
    const terminal = page.locator(".xterm");
    await expect(terminal).toBeVisible();
    await expect(page.getByTestId("terminal-touch-bar")).toBeVisible();
    await terminal.click();
    const rows = page.locator(".xterm-rows");

    // Up sends the arrow's keys: `cat -v` shows them as ^[[A (whatever the
    // shell: CI's /bin/sh keeps no line history to recall).
    await page.keyboard.type("cat -v");
    await page.keyboard.press("Enter");
    await page.getByTestId("terminal-key-up").tap();
    await page.keyboard.press("Enter");
    await expect(rows).toContainText("^[[A");

    // Ctrl, then c, interrupts the running command.
    await page.getByTestId("terminal-key-ctrl").tap();
    await expect(page.getByTestId("terminal-key-ctrl")).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.type("c");
    await expect(page.getByTestId("terminal-key-ctrl")).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.type("echo after-$((6*7))");
    await page.keyboard.press("Enter");
    await expect(rows).toContainText("after-42");

    // Compose puts its text in the terminal and presses no Enter.
    await page.getByTestId("terminal-compose").tap();
    await page.getByTestId("terminal-compose-text").fill("echo composed-$((6*7))");
    await page.getByTestId("terminal-compose-insert").tap();
    await expect(page.getByTestId("terminal-compose-text")).not.toBeVisible();
    await expect(rows).not.toContainText("composed-42");
    await terminal.click();
    await page.keyboard.press("Enter");
    await expect(rows).toContainText("composed-42");
  } finally {
    await tablet.context.close();
  }
});

test("a mouse and keyboard get no touch bar", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=terminal");
  await expect(page.locator(".xterm")).toBeVisible();
  await expect(shown(page).getByTestId("terminal-touch-bar")).toHaveCount(0);
});

/** A finger's swipe over `target`, from (x, y0) to (x, y1), as Chromium's
 *  touch input sends it (a real touch, not synthetic DOM events). */
async function swipe(page: import("@playwright/test").Page, target: import("@playwright/test").Locator, from: number, to: number) {
  const box = (await target.boundingBox())!;
  const x = box.x + box.width / 2;
  const cdp = await page.context().newCDPSession(page);
  const at = (y: number) => [{ x, y: box.y + y }];
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: at(from) });
  const steps = 12;
  for (let i = 1; i <= steps; i++) {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: at(from + ((to - from) * i) / steps) });
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
}

// A finger scrolls the terminal: a swipe down shows the lines above.
test("a swipe scrolls a shell's terminal back", async ({ browser, baseURL, daemon }) => {
  test.skip(browser.browserType().name() !== "chromium", "touch input is Chromium's");
  const tablet = await client(browser, baseURL!, daemon, { width: 1180, height: 820 }, { hasTouch: true });
  const page = tablet.page;
  try {
    await open(page, daemon, "oxplow:oxplow/page?page=terminal");
    const terminal = page.locator(".xterm");
    await expect(terminal).toBeVisible();
    await terminal.click();
    const rows = page.locator(".xterm-rows");
    await page.keyboard.type("seq -f 'line-%g' 1 300");
    await page.keyboard.press("Enter");
    await expect(rows).toContainText("line-300");
    await expect(rows).not.toContainText("line-200 ");
    for (let i = 0; i < 6; i++) await swipe(page, terminal, 60, 560);
    await expect(rows).not.toContainText("line-300");
  } finally {
    await tablet.context.close();
  }
});
