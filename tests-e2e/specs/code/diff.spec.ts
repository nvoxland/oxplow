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
  await page.getByTestId("commit-graph-row").filter({ hasText: "Add the emu file" }).click();
  await expect(page.getByTestId("page-git-commit")).toBeVisible();
  // Its Change Analysis lenses read their extension's models (tsk977):
  // once the slot has rendered its runs, none failed.
  await expect(page.getByTestId("page-git-commit")).toContainText("1 file");
  await expect(page.getByTestId("vcs.commit.details-oxplow-analytics/change-review")).toBeVisible();
  await expect(page.getByTestId("page-git-commit")).not.toContainText("no such table");
  await page.getByTestId("git-commit-files").getByText("emu.txt").click();
  await expect(page.getByTestId("page-diff")).toContainText("second emu line");
});
