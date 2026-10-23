import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

test("a committed change shows its hunk", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "first emu line\nsecond emu line\n");
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  git("add", "emu.txt");
  git("commit", "-q", "-m", "Add the emu file");
  await page.goto("/");
  await openFromLauncher(page, "Git History");
  // Git History is oxplow-bundled's graph, in its sandboxed frame.
  await page
    .frameLocator('[data-testid="custom-component-frame"]')
    .locator(".row", { hasText: "Add the emu file" })
    .click();
  // The commit is the foundation's `commit` lens page.
  const commit = page.locator('[data-testid="page-lens"]:visible');
  await expect(commit).toContainText("Add the emu file");
  // Its Change Analysis lenses (oxplow-bundled's mounts in the page's
  // `.sections` slot) read their extension's models (tsk977): once they
  // have rendered their runs, none failed.
  await expect(commit.locator(".ox-section-title").getByText("Change Analysis")).toBeVisible();
  await expect(commit).not.toContainText("no such table");
  await commit.getByTestId("lens-part-files").getByRole("button", { name: "emu.txt" }).click();
  await expect(page.getByTestId("lens-diff")).toContainText("second emu line");
  // The diff is the foundation's `diff` lens page. A diff is its address: a reload draws the same diff, its base named.
  const named = await page.getByTestId("lens-diff").getByText(/^vs /).textContent();
  await page.reload();
  await expect(page.getByTestId("lens-diff")).toContainText("second emu line");
  await expect(page.getByTestId("lens-diff").getByText(/^vs /)).toHaveText(named!);
});
