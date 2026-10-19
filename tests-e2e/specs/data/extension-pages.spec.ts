import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher } from "../../support/ui.js";

test("an extension's page is offered, and turning the extension off takes it away", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  // The test extension's `item` page, in the launcher's Work pages.
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-page:e2e/item")).toBeVisible();
  await page.keyboard.press("Escape");
  // The Extensions list (the foundation's lens in Settings): Disable is on
  // the row's right-click menu.
  await openFromLauncher(page, "Settings: Extensions");
  const row = page.locator('[data-testid^="lens-row-"]', { hasText: "e2e" }).first();
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-disable").click();
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-page:e2e/item")).toHaveCount(0);
});
