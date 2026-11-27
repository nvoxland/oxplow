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
  // Other toasts may be up or come after (the start's own, a project's
  // language-pack suggestion), and only the newest is `undo-toast`: the
  // close's is found by its words, and its Undo taken.
  const closed = page.locator("[data-toast-id]").filter({ hasText: "Close" });
  await expect(closed).toBeVisible();
  await closed.getByRole("button", { name: "Undo" }).click();
  await expect(tabOf(page, id)).toBeVisible();
});
