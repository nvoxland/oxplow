import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

test("a committed change shows its hunk", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "first emu line\nsecond emu line\n");
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  git("add", "emu.txt");
  git("commit", "-q", "-m", "Add the emu file");
  await open(page, daemon, "oxplow:oxplow/page?page=git-history");
  await page.locator('[data-node="history"]:visible').getByText("Add the emu file").click();
  // The commit is the foundation's `commit` page.
  const commit = shown(page).getByTestId("page-file");
  await expect(commit).toContainText("Add the emu file");
  await commit.locator('[data-node="files"]').getByRole("button", { name: "emu.txt" }).click();
  await expect(page.getByTestId("viz-diff")).toContainText("second emu line");
  // The diff is the foundation's `diff` page. A diff is its address: a reload draws the same diff, its base named.
  const named = await page.getByTestId("viz-diff").getByText(/^vs /).textContent();
  await page.reload();
  await expect(page.getByTestId("viz-diff")).toContainText("second emu line");
  await expect(page.getByTestId("viz-diff").getByText(/^vs /)).toHaveText(named!);
});
