import { run, waitForModels } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, openFromLauncher, openNewTask } from "../../support/ui.js";

// On a daemon of their own: the rail's Up Next shows the thread's first ten
// ready tasks, which other specs' tasks would otherwise fill.
test("a task made in the UI is on the thread's list", async ({ page }) => {
  await page.goto("/");
  await openNewTask(page);
  const form = page.locator('[data-testid="viz-form"]:visible');
  await form.getByTestId("viz-form-fields-title").fill("Made in the UI");
  await form.getByTestId("viz-form-fields-submit").click();
  // The form gives way to the item it made, whose own page has forms too.
  await expect(page.getByTestId("viz-form-fields-again")).toHaveCount(0);
  await expandRailSection(page, "ext:oxplow-bundled/work");
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toContainText("Made in the UI");
});

// The New Item form asks for the active list's own fields, and its second
// submit files the item and starts the next with the list's fields kept.
test("Save and Another keeps the state and the list's fields", async ({ page }) => {
  await page.goto("/");
  await openNewTask(page);
  const form = page.locator('[data-testid="viz-form"]:visible');
  await form.getByTestId("viz-form-fields-native.priority").selectOption("high");
  await form.getByTestId("viz-form-fields-title").fill("First of two");
  await form.getByTestId("viz-form-fields-again").click();
  await expect(form.getByTestId("viz-form-fields-title")).toHaveValue("");
  await expect(form.getByTestId("viz-form-fields-native.priority")).toHaveValue("high");
  await form.getByTestId("viz-form-fields-title").fill("Second of two");
  await form.getByTestId("viz-form-fields-submit").click();
  await expandRailSection(page, "ext:oxplow-bundled/work");
  const work = page.getByTestId("rail-section-ext:oxplow-bundled/work");
  await expect(work).toContainText("First of two");
  await expect(work).toContainText("Second of two");
});

test("a task made elsewhere appears without a reload", async ({ page, daemon }) => {
  await page.goto("/");
  await expandRailSection(page, "ext:oxplow-bundled/work");
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toBeVisible();
  await waitForModels(daemon, ["v_work_item"], () =>
    run(daemon, "oxplow.work_item.create", { title: "Made elsewhere", thread: daemon.thread }),
  );
  await expect(page.getByTestId("rail-section-ext:oxplow-bundled/work")).toContainText("Made elsewhere");
});

// The Tasks page's scope is its view params: in its tab's id, so a reload
// keeps it, and opening Tasks again while one tab shows another scope
// focuses that tab at the default view.
test("the Tasks page's scope is its tab's: it re-renders the page, a reload keeps it", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  const body = () => page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid*="page=tasks"]:visible');
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=tasks");
  // A mark on the page's element: a change of view re-renders the page,
  // it doesn't mount it again.
  await body().getByTestId("page-file").evaluate((el) => {
    el.setAttribute("data-mounted-once", "yes");
  });
  await page.getByTestId("page-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-show-edit").filter({ visible: true }).selectOption("all");
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=tasks#show=all");
  await expect(body().getByTestId("page-file")).toHaveAttribute("data-mounted-once", "yes");
  await page.reload();
  await expect(body()).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=tasks#show=all");
  await expect(page.getByTestId("page-param-show-value").filter({ visible: true })).toContainText("Everything");
  // Tasks at its default view is the same page: the one tab shows it, no
  // second tab opens.
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/page?page=tasks")).toBeVisible();
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/page?page=tasks"]')).toHaveCount(1);
});

// Tasks groups the thread's items by state; a row's menu changes one, and
// a parent's children fold away under it.
test("the Tasks page lists the thread's items by state, with epics that fold", async ({ page, daemon }) => {
  const epic = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", { title: "The epic", thread: daemon.thread });
  await run(daemon, "oxplow.work_item.create", { title: "Under the epic", thread: daemon.thread, parent_uri: epic.result.uri });
  await run(daemon, "oxplow.work_item.create", { title: "Standalone", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  const list = page.locator('[data-node="list"]');
  await expect(list).toContainText("Under the epic");
  await list.locator('[data-testid^="viz-row-fold-"]').first().click();
  await expect(list).not.toContainText("Under the epic");
  await expect(list).toContainText("Standalone");
  await list.locator('[data-testid^="viz-row-"]').filter({ hasText: "Standalone" }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-to-blocked").click();
  await expect(page.locator('[data-node="summary"]')).toContainText("Blocked");
  // Rows sit under their state's heading and count: the item is now under Blocked.
  await expect(page.locator('[data-node="list"]')).toContainText(/Blocked1\S*Standalone/);
});
