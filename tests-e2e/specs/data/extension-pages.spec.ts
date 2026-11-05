import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher } from "../../support/ui.js";

test("an extension's page is offered, and turning the extension off takes it away", async ({ page }) => {
  await page.goto("/");
  // The test extension's `overview` page, in the launcher's Work pages.
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).toBeVisible();
  await page.keyboard.press("Escape");
  // The Extensions list (the foundation's page) opens an extension's own
  // page, which has Disable among its actions.
  await openFromLauncher(page, "Extensions");
  await page.locator('[data-testid^="viz-row-"]').filter({ visible: true }).getByText("e2e", { exact: true }).first().click();
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-action-disable").click();
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).toHaveCount(0);
});
