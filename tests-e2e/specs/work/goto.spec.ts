import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openFromLauncher } from "../../support/ui.js";

// The rail's Go To is oxplow-bundled's panel over `v_bookmark`
// and `v_page_visit`: a page starred from its nav bar shows there, and a
// page visited shows in its history.
test("a starred page is in the rail's Go To", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  await page.getByTestId("page-nav-bookmark").click();
  await page.getByTestId("page-nav-bookmark-scope-thread").click();
  await expandRailSection(page, "ext:oxplow-bundled/go-to");
  const goTo = page.getByTestId("rail-section-ext:oxplow-bundled/go-to");
  await expect(goTo).toContainText("Bookmarks");
  await expect(goTo).toContainText("Metrics");
  await expect(goTo).toContainText("History");
});
