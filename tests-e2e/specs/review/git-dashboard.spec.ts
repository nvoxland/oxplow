import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Git Dashboard is the foundation's document lens: the stream's branch
// against its upstream, the working tree and the recent commits.
test("the Git Dashboard draws its sections for the stream", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Git Dashboard");
  const doc = page.locator('[data-testid="lens-part-upstream"]:visible');
  await expect(doc).toBeVisible();
  await expect(page.locator('[data-testid="lens-part-recent"]:visible')).toBeVisible();
});
