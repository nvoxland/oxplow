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
  await expect(page.locator('[data-testid="lens-part-tiles"]:visible')).toContainText("No tiles yet");
});

// The dashboard is the foundation's lens: its actions add a heading tile
// (a one-field form, so a prompt strip) and rename or delete the dashboard.
test("a dashboard takes a heading tile, a new name, and is deleted from its page", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Dashboards");
  await page.getByRole("button", { name: "New Dashboard" }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-testid="lens-part-tiles"]:visible')).toBeVisible();
  await page.getByRole("button", { name: "Add Heading…" }).filter({ visible: true }).first().click();
  await page.locator('[data-testid="lens-action-form-fields-text"]:visible').fill("Build health");
  await page.keyboard.press("Enter");
  await expect(page.locator('[data-testid="lens-part-tiles"]:visible')).toContainText("Build health");
  await page.getByRole("button", { name: "Rename…" }).filter({ visible: true }).first().click();
  const title = page.locator('[data-testid="lens-action-form-fields-title"]:visible');
  // Rename… starts from the dashboard's name: the lens's one row.
  await expect(title).toHaveValue("Untitled dashboard");
  await title.fill("Health");
  await page.keyboard.press("Enter");
  await expect(page.locator('[data-testid="lens-part-title"]:visible')).toContainText("Health");
  await page.getByRole("button", { name: "Delete" }).filter({ visible: true }).first().click();
  // Deleting asks first; confirmed, the dashboard is gone and its page closes.
  await page.getByTestId("lens-action-confirm-run").click();
  await expect(page.locator('[data-testid="lens-part-tiles"]:visible')).toHaveCount(0);
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
