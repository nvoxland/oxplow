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
  await expect(page.getByTestId("action-today")).toBeVisible();
  await expect(page.getByTestId("action-fit")).toBeVisible();
  await page.getByTestId("action-fit").click();
  await expect(frame.locator(".vis-item").first()).toBeVisible();
});
