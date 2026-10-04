import { run, waitForModels } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openNewTask } from "../../support/ui.js";

test("a task made in the UI is on the thread's list", async ({ page }) => {
  await page.goto("/");
  await openNewTask(page);
  await page.getByTestId("tasks-title").fill("Made in the UI");
  await page.getByTestId("tasks-save").click();
  await expandRailSection(page, "core:work");
  await expect(page.getByTestId("rail-section-core:work")).toContainText("Made in the UI");
});

test("a task made elsewhere appears without a reload", async ({ page, daemon }) => {
  await page.goto("/");
  await expandRailSection(page, "core:work");
  await expect(page.getByTestId("rail-section-core:work")).toBeVisible();
  await waitForModels(daemon, ["v_task"], () =>
    run(daemon, "work_item.create", { title: "Made elsewhere", native: { thread: daemon.thread } }),
  );
  await expect(page.getByTestId("rail-section-core:work")).toContainText("Made elsewhere");
});
