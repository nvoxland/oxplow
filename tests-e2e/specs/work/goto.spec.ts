import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openFromLauncher } from "../../support/ui.js";

// The rail's Go To is oxplow-bundled's panel over `v_bookmark`
// and `v_page_visit`: a page starred from its nav bar shows there, and a
// page visited shows in its history.
test("a starred page is in the rail's Go To", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  // Every open page has a nav bar; the visible one is Metrics'.
  await page.locator('[data-testid="page-nav-bookmark"]:visible').click();
  await page.locator('[data-testid="page-nav-bookmark-scope-thread"]:visible').click();
  await expandRailSection(page, "ext:oxplow-bundled/go-to");
  const goTo = page.getByTestId("rail-section-ext:oxplow-bundled/go-to");
  await expect(goTo).toContainText("Bookmarks");
  await expect(goTo).toContainText("Metrics");
  await expect(goTo).toContainText("History");
});

// Go To is the foundation's lens page: the starred page is a bookmark row
// there, and Recently visited lists pages.
test("the Go To page lists a starred page and the pages visited", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Metrics");
  await page.locator('[data-testid="page-nav-bookmark"]:visible').click();
  await page.locator('[data-testid="page-nav-bookmark-scope-thread"]:visible').click();
  await openFromLauncher(page, "Go To");
  const goTo = page.locator('[data-section="bookmarks"]:visible');
  await expect(goTo).toContainText("Metrics");
  await expect(page.locator('[data-section="recent"]:visible')).not.toContainText("No visits yet");
});

// New Dashboard makes an empty dashboard and opens it, as the command's
// own `open_after` says.
test("New Dashboard on the Dashboards page opens the new dashboard", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Dashboards");
  await page.getByRole("button", { name: "New Dashboard" }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-testid="page-custom-dashboard"]:visible')).toBeVisible();
});

// Local History is the foundation's document of two lenses: its snapshots
// recent, or by git commit (the `mode` param, in the address).
test("the Local History page draws its snapshots and keeps its grouping in the address", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Local History");
  const body = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid$="page=local-history"]:visible');
  await expect(body.getByRole("heading", { level: 1 })).toHaveText("Local History");
  await page.getByTestId("lens-param-mode-value").filter({ visible: true }).click();
  await page.getByTestId("lens-param-mode-edit").filter({ visible: true }).selectOption("by-commit");
  await expect(page.locator('[data-testid="center-tab-body-oxplow:oxplow/page?page=local-history#mode=by-commit"]:visible')).toHaveCount(1);
});
