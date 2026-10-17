import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// A session's tab closes the session on one click of its × — in WebKit
/// too — and the toast's Undo opens it again, its tab back. Closing a
/// thread's last session leaves its other tabs, not the picker.
test("a session's tab closes the session at once, and Undo brings it back", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "New Agent Session");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-start").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await expect(page.getByTestId(tabTestId)).toHaveCount(0);
  await expect(page.getByTestId("page-new-session")).toHaveCount(0);
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(tabTestId)).toBeVisible();
});
