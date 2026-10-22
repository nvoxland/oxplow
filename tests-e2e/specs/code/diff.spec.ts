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
  await expect(commit.getByText("Change Analysis")).toBeVisible();
  await expect(commit).not.toContainText("no such table");
  await commit.getByRole("button", { name: "emu.txt" }).click();
  await expect(page.getByTestId("page-diff")).toContainText("second emu line");
  // A diff is its address: a reload draws the same diff, its base named.
  const named = await page.getByTestId("page-diff").getByText(/^vs /).textContent();
  await page.reload();
  await expect(page.getByTestId("page-diff")).toContainText("second emu line");
  await expect(page.getByTestId("page-diff").getByText(/^vs /)).toHaveText(named!);
});
