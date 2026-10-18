import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The recorded-metrics page groups metrics in kit sections whose headings
// toggle them; Collapse all / Expand all act on the ones shown.
test("the Metrics page's sections collapse and expand", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  const toggles = page.locator('[data-testid^="recorded-section-toggle-"]:visible');
  await expect(toggles.first()).toBeVisible();
  const first = toggles.first();
  await expect(first).toHaveAttribute("aria-expanded", "true");
  await first.click();
  await expect(first).toHaveAttribute("aria-expanded", "false");
  await page.locator('[data-testid="recorded-collapse-all"]:visible').click();
  for (const t of await toggles.all()) await expect(t).toHaveAttribute("aria-expanded", "false");
  await page.locator('[data-testid="recorded-expand-all"]:visible').click();
  for (const t of await toggles.all()) await expect(t).toHaveAttribute("aria-expanded", "true");
});
