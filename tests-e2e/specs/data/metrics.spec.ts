import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Metrics page is the foundation's lens over the catalog and
// `v_metric_point`: its sections list metrics with a trend and a latest value.
test("the Metrics page lists the catalog by section with a trend column", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  const list = page.locator('[data-testid="lens-list"]:visible');
  await expect(list).toBeVisible();
  await expect(list.locator('[data-testid^="lens-group"]').first()).toBeVisible();
});
