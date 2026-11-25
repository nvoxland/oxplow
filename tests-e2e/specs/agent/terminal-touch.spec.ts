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

    // Up recalls the last line from the shell's history.
    await page.keyboard.type("echo recalled-$((6*7))");
    await page.keyboard.press("Enter");
    await expect(rows).toContainText("recalled-42");
    await page.getByTestId("terminal-key-up").tap();
    await page.keyboard.press("Enter");
    await expect.poll(async () => ((await rows.textContent()) ?? "").split("recalled-42").length - 1).toBeGreaterThanOrEqual(2);

    // Ctrl, then c, interrupts a running command.
    await page.keyboard.type("sleep 30");
    await page.keyboard.press("Enter");
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
