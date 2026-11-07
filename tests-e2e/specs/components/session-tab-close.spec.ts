import { expect, test } from "../../support/fixtures.js";
import { newSessionPage, open } from "../../support/ui.js";

/// A session's tab closes the session on one click of its × — in WebKit
/// too — and the toast's Undo opens it again, its tab back. Closing a
/// thread's last session leaves its other tabs, not the picker.
test("a session's tab closes the session at once, and Undo brings it back", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=new-session");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await expect(page.getByTestId(tabTestId)).not.toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  // The start's own toast is still up: the latest toast is the close's only
  // once its run has come back, and its Undo is the one to take.
  await expect(page.getByTestId("undo-toast")).toContainText("Close");
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(tabTestId)).toBeVisible();
});
