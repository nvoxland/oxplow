import { expect, test } from "../../support/fixtures.js";

test("the app boots against its daemon: the shell renders and no call fails", async ({ page }) => {
  const failed: string[] = [];
  page.on("requestfailed", (r) => failed.push(`${r.method()} ${r.url()}: ${r.failure()?.errorText}`));
  page.on("response", (r) => {
    if (r.url().includes("/ipc/") && !r.ok()) failed.push(`${r.status()} ${r.url()}`);
  });
  await page.goto("/");
  await expect(page.getByTestId("rail-hud")).toBeVisible();
  await expect(page.getByTestId("status-bar-context")).toBeVisible();
  await expect(page.getByTestId("page-agent")).toBeVisible();
  expect(failed).toEqual([]);
});
