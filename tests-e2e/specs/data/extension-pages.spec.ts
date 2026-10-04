import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

test("an extension's page is offered, and turning the extension off takes it away", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  // The test extension's `item` page, in the launcher's Work pages.
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("Item");
  await expect(page.getByTestId("launcher-page-page:ext.e2e.item")).toBeVisible();
  await page.keyboard.press("Escape");
  await openFromLauncher(page, "Settings");
  const toggle = page.getByTestId("extension-toggle-e2e");
  await toggle.click();
  await expect(page.getByTestId("extension-row-e2e")).toContainText(/off|disabled/i);
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("Item");
  await expect(page.getByTestId("launcher-page-page:ext.e2e.item")).toHaveCount(0);
});
