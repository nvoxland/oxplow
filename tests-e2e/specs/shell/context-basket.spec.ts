import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A page's ☰ → Add to Context puts the page in the selected thread's
// basket: the dock's Context icon counts it, its panel lists it, and the
// × takes it out again.
test("a page added from its ☰ is counted in the dock and removed from its panel", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Alerts");
  await page.locator('[data-testid="page-nav-menu"]:visible').click();
  await page.getByTestId("menu-item-page-add-to-context").click();

  const badge = page.getByTestId("dock-context-badge");
  await expect(badge).toHaveText("1");

  await page.getByTestId("dock-context").click();
  await expect(page.getByTestId("context-panel-item")).toHaveCount(1);
  await page.getByTestId("context-panel-remove").click();
  await expect(page.getByTestId("context-panel-item")).toHaveCount(0);
  await expect(badge).toHaveCount(0);
  await expect(page.getByTestId("context-panel-empty")).toBeVisible();
});
