import { expect, test } from "../../support/fixtures.js";
import { newSessionPage, open, closeTab, tabsWith, tabUri, tabOf } from "../../support/ui.js";

/// A session's tab closes the session on one click of its × — in WebKit
/// too — and the toast's Undo opens it again, its tab back. Closing a
/// thread's last session leaves its other tabs, not the picker.
test("a session's tab closes the session at once, and Undo brings it back", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=new-session");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const tab = tabsWith(page, "oxplow:oxplow/agent_session?").first();
  await expect(tab).toBeVisible();
  const id = await tabUri(tab);
  await closeTab(page, id);
  await expect(tabOf(page, id)).not.toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  // Other toasts may be up (the start's own, a project's language-pack
  // suggestion): the close's is the one whose Undo to take.
  const closed = page.getByTestId("undo-toast").filter({ hasText: "Close" });
  await expect(closed).toBeVisible();
  await closed.getByTestId("undo-toast-undo").click();
  await expect(tabOf(page, id)).toBeVisible();
});
