import { approveProgram } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The suite's threads run the fake ACP agent, which does what each
// `fake:<step>` line of a prompt says.

test("the fake agent's reply streams into the thread's transcript", async ({ page, daemon }) => {
  // An ACP agent is a project program: it runs once a person approves it.
  await approveProgram(daemon, "acp-agent", "fake");
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
});
