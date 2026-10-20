import { run, waitForModels } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openFromLauncher, openNewTask } from "../../support/ui.js";

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

// The Tasks page's scope is its view params: in its tab's id, so Back
// undoes a change, a reload keeps it, and opening Tasks again while one
// tab shows another scope opens a second tab beside it.
test("the Tasks page's scope is its tab's: it re-renders the page, a reload keeps it", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  const body = () => page.locator('[data-testid^="center-tab-body-page:tasks"]:visible');
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-page:tasks");
  // A mark on the page's element: a change of view re-renders the page,
  // it doesn't mount it again.
  await body().getByTestId("tasks-scope-picker").evaluate((el) => {
    el.setAttribute("data-mounted-once", "yes");
  });
  await body().getByTestId("tasks-scope-button").click();
  await body().getByTestId("tasks-scope-all").click();
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-page:tasks?scope=all");
  await expect(body().getByTestId("tasks-scope-picker")).toHaveAttribute("data-mounted-once", "yes");
  // A scope is a filter: it replaced the entry, so there's no Back to it.
  await expect(page.getByTestId("page-nav-back").filter({ visible: true })).toBeDisabled();
  await page.reload();
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-page:tasks?scope=all");
  await expect(body().getByTestId("tasks-scope-button")).toContainText("Everything");
  // Tasks at its default view is another page of the tab strip.
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("center-tab-page:tasks")).toBeVisible();
  await expect(page.getByTestId("center-tab-page:tasks?scope=all")).toBeVisible();
});
