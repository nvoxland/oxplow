import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { ipc, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Git History's commits drawn as a graph: oxplow-bundled's `history-graph`
// component, replacing oxplow's plain list (`vcs.history.graph`) — a
// bundled extension's component, served from its embedded files, in
// Chromium and WebKit.

/** Commit `n` files on a side branch and merge it, so the graph has a
 *  second lane. */
function history(daemon: Daemon) {
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  writeFileSync(join(daemon.project, "main.txt"), "main\n");
  git("add", "main.txt");
  git("commit", "-q", "-m", "Main work");
  git("checkout", "-q", "-b", "side");
  writeFileSync(join(daemon.project, "side.txt"), "side\n");
  git("add", "side.txt");
  git("commit", "-q", "-m", "Side work");
  git("checkout", "-q", "-");
  git("merge", "-q", "--no-ff", "-m", "Merge side", "side");
}

test("the history graph draws lanes, filters and opens a commit", async ({ fresh }) => {
  const { page, daemon } = fresh;
  history(daemon);
  await page.goto("/");
  await openFromLauncher(page, "Git History");
  await expect(page.getByTestId("replacement-vcs.history.graph")).toContainText("replaced by oxplow-bundled");
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  await expect(frame.locator(".row", { hasText: "Merge side" })).toBeVisible();
  // The merge's second parent is on a lane of its own.
  await expect(frame.locator(".row", { hasText: "Side work" }).locator("circle.node-1")).toHaveCount(1);

  await frame.locator("#query").fill("side work");
  await expect(frame.locator(".row:not(.dimmed)")).toHaveCount(1);
  await frame.locator("#query").fill("");

  await frame.locator(".row", { hasText: "Main work" }).click();
  await expect(page.getByTestId("page-git-commit")).toContainText("Main work");
});

test("without oxplow-bundled, Git History is oxplow's plain list", async ({ fresh }) => {
  const { page, daemon } = fresh;
  history(daemon);
  await ipc(daemon, "set_extension_enabled", { name: "oxplow-bundled", enabled: false });
  await page.goto("/");
  await openFromLauncher(page, "Git History");
  await expect(page.getByTestId("commit-list-row").filter({ hasText: "Merge side" })).toBeVisible();
  await expect(page.locator('[data-testid^="replacement-"]')).toHaveCount(0);
});
