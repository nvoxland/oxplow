import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher, load } from "../../support/ui.js";

test("an extension's page is offered, and turning the extension off takes it away", async ({ page, daemon }) => {
  await load(page);
  // The test extension's `overview` page, in the launcher's Work pages.
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).toBeVisible();
  await page.keyboard.press("Escape");
  // Disabling it (the command the Extensions page's Disable runs) updates the open launcher's pages.
  await run(daemon, "oxplow.extension.disable", { name: "e2e" });
  await searchLauncher(page, "Item");
  await expect(page.getByTestId("launcher-page-oxplow:oxplow/page?page=e2e/overview")).not.toBeVisible();
});
