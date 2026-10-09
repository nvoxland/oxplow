import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// A session's tab closes the session once its × is armed and its "Close
/// session" confirm clicked — in WebKit too, where clicking a button
/// doesn't focus it, so focus lands on the tab around it. Closing a
/// thread's last session leaves its other tabs, not the picker.
test("a session's tab closes the session once its close is confirmed", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "New Agent Session");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-start").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await page.getByTestId(`center-tab-close-${id}-confirm`).click();
  await expect(page.getByTestId(tabTestId)).toHaveCount(0);
  await expect(page.getByTestId("page-new-session")).toHaveCount(0);
});
