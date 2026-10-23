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

// A metric's row opens its page: the foundation's `metric` lens, a header
// with the definition over the trend and the recordings.
test("a metric on the Metrics page opens its page with the definition", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  await page.getByRole("button", { name: "Coverage", exact: true }).filter({ visible: true }).click();
  const header = page.locator('[data-testid="lens-part-header"]:visible');
  await expect(header).toContainText("Aggregation");
  await expect(header).toContainText("Disable");
  // A fresh project has recorded nothing yet.
  await expect(page.getByTestId("lens-document-nothing-found")).toContainText("Recordings");
});
