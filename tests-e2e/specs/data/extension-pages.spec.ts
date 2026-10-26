import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher } from "../../support/ui.js";

test("an extension's page is offered, and turning the extension off takes it away", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  // The test extension's `overview` page, in the launcher's Work pages.
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).toBeVisible();
  await page.keyboard.press("Escape");
  // The Extensions list (the foundation's page in Settings): Disable is on
  // the row's right-click menu.
  await openFromLauncher(page, "Settings: Extensions");
  const row = page.locator('[data-testid^="viz-row-"]', { hasText: "e2e" }).first();
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-disable").click();
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).toHaveCount(0);
});
