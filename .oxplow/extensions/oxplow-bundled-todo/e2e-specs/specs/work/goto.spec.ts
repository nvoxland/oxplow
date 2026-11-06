import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openFromLauncher } from "../../support/ui.js";

// The rail's Go To is oxplow-bundled-todo's panel over `v_bookmark`
// and `v_page_visit`: a page starred from its nav bar shows there, and a
// page visited shows in its history.
test("a starred page is in the rail's Go To", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  // Every open page has a nav bar; the visible one is Metrics'.
  await page.locator('[data-testid="page-nav-bookmark"]:visible').click();
  await page.locator('[data-testid="page-nav-bookmark-scope-thread"]:visible').click();
  await expandRailSection(page, "ext:oxplow-bundled-todo/go-to");
  const goTo = page.getByTestId("rail-section-ext:oxplow-bundled-todo/go-to");
  await expect(goTo).toContainText("Bookmarks");
  await expect(goTo).toContainText("Metrics");
  await expect(goTo).toContainText("History");
});
