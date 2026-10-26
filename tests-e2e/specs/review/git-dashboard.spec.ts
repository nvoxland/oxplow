import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Git Dashboard is the foundation's page `git-dashboard`, titled
// "Git": the stream's branch against its upstream, the working tree
// and the recent commits.
test("the Git Dashboard draws its sections for the stream", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Git");
  const doc = page.locator('[data-testid="page-file"]:visible [data-node="upstream"]');
  await expect(doc).toBeVisible();
  await expect(page.locator('[data-testid="page-file"]:visible [data-node="recent"]')).toBeVisible();
});
