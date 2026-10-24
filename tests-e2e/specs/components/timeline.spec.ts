import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Timeline (tsk1455): oxplow-foundation's custom page, a component
// built with a library the kit doesn't have, filling its page under core's
// nav bar. It reads with its own SQL, titles its page with the range and
// puts Today and Fit in the bar, which core draws.
test("the Timeline page draws the project's activity and titles itself", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Timeline");
  await expect(page.getByTestId("page-custom")).toBeVisible();
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  // A fresh project has its first thread and session in the last week.
  await expect(frame.locator(".vis-item").first()).toBeVisible();
  await expect(page.getByTestId("page-nav-title")).toContainText("Timeline:");
  // Today and Fit are prominent: buttons in the bar, not in its ☰.
  await expect(page.getByTestId("action-today")).toBeVisible();
  await expect(page.getByTestId("action-fit")).toBeVisible();
  await page.getByTestId("action-fit").click();
  await expect(frame.locator(".vis-item").first()).toBeVisible();
});

// Its window is its view params: Fit writes `from` and `to` into the tab's
// address (a filter: it replaces the entry), and a reload shows the same
// window.
test("the Timeline's window is its tab's: Fit puts it in the address, a reload keeps it", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Timeline");
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  await expect(frame.locator(".vis-item").first()).toBeVisible();
  // The window it opens at is "the last week", not a date: not in the address.
  const body = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid*="page=timeline"]:visible');
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=timeline");
  await expect(page.getByTestId("page-nav-title")).toContainText("Timeline:");
  const opened = await page.getByTestId("page-nav-title").textContent();
  await page.getByTestId("action-fit").click();
  await expect(body).toHaveAttribute("data-testid", /from=.*to=|to=.*from=/);
  await expect(page.getByTestId("page-nav-back").filter({ visible: true })).toBeDisabled();
  // The page titles itself with the window it loaded: Fit's, once loaded.
  await expect(page.getByTestId("page-nav-title")).not.toHaveText(opened!);
  const title = await page.getByTestId("page-nav-title").textContent();
  const id = await body.getAttribute("data-testid");
  await page.reload();
  await expect(body).toHaveAttribute("data-testid", id!);
  await expect(page.getByTestId("page-nav-title")).toHaveText(title!);
});
