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
  await page.goto("/");
  await openFromLauncher(page, "Terminal");
  const terminal = page.locator(".xterm");
  await expect(terminal).toBeVisible();
  await terminal.click();
  // The output, not the typed line: only the shell computes 42.
  await page.keyboard.type("echo e2e-$((6*7))");
  await page.keyboard.press("Enter");
  await expect(page.locator(".xterm-rows")).toContainText("e2e-42");
  // It fits its pane: the screen's last row is inside the page, not
  // clipped below it (tsk1042).
  const screen = await page.locator(".xterm-screen").boundingBox();
  const pageBox = await page.getByTestId("page-terminal").boundingBox();
  expect(screen && pageBox && screen.y + screen.height <= pageBox.y + pageBox.height + 1).toBeTruthy();
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
