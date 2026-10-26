import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Metrics page groups its metrics under section headings that fold.
test("the Metrics page's sections collapse and expand", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  const toggle = page.locator('[data-testid="viz-group-heading"]:visible button[aria-expanded]').first();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
});
