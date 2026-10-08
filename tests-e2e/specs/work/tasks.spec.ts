import { run, waitForModels } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openNewTask } from "../../support/ui.js";

// On a daemon of their own: the rail's Up Next shows the thread's first ten
// ready tasks, which other specs' tasks would otherwise fill.
test("a task made in the UI is on the thread's list", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openNewTask(page);
  await page.getByTestId("tasks-title").fill("Made in the UI");
  await page.getByTestId("tasks-save").click();
  await expandRailSection(page, "ext:oxplow-bundled/work");
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toContainText("Made in the UI");
});

test("a task made elsewhere appears without a reload", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await expandRailSection(page, "ext:oxplow-bundled/work");
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toBeVisible();
  await waitForModels(daemon, ["v_work_item"], () =>
    run(daemon, "oxplow.work_item.create", { title: "Made elsewhere", thread: daemon.thread }),
  );
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toContainText("Made elsewhere");
});
