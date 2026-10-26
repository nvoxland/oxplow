import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Backlog is the foundation's `backlog` document: a quick-add form over
// a list of the items on no thread, grouped by state, with row actions,
// bulk changes and drag to reorder.
const rowOf = (page: import("@playwright/test").Page, title: string) =>
  page.locator('[data-node="items"]').locator('[data-testid^="viz-row-"]').filter({ hasText: title });

test("an item typed into the form is filed on the backlog, and the thread's items aren't listed", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Already waiting" });
  await run(daemon, "oxplow.work_item.create", { title: "On the thread", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Backlog");
  const items = page.locator('[data-node="items"]');
  await expect(items).toContainText("Already waiting");
  await expect(items).not.toContainText("On the thread");

  await page.locator('[data-node="add"]').getByTestId("viz-form-fields-title").fill("Filed from the form");
  await page.keyboard.press("Enter");
  await expect(items).toContainText("Filed from the form");
});

test("a row's menu changes its state and a marked pair changes together", async ({ page, daemon }) => {
  for (const title of ["First waiting", "Second waiting", "Third waiting"]) await run(daemon, "oxplow.work_item.create", { title });
  await page.goto("/");
  await openFromLauncher(page, "Backlog");

  await rowOf(page, "First waiting").click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-to-blocked").click();
  await expect(page.locator('[data-node="items"]')).toContainText("blocked");

  await rowOf(page, "Second waiting").click({ modifiers: ["Meta"] });
  await rowOf(page, "Third waiting").click({ modifiers: ["Meta"] });
  await page.getByTestId("viz-bulk-to-done").click();
  await expect(rowOf(page, "Second waiting")).toContainText("✓");
  await expect(rowOf(page, "Third waiting")).toContainText("✓");
});

test("Bring to this thread moves an item off the backlog", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Pull me in" });
  await page.goto("/");
  await openFromLauncher(page, "Backlog");
  await rowOf(page, "Pull me in").click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-bring").click();
  await expect(page.locator('[data-node="items"]')).not.toContainText("Pull me in");
});

test("dragging a row above another reorders the list", async ({ page, daemon }) => {
  for (const title of ["Alpha item", "Beta item", "Gamma item"]) await run(daemon, "oxplow.work_item.create", { title });
  await page.goto("/");
  await openFromLauncher(page, "Backlog");
  const order = async () => (await page.locator('[data-node="items"]').locator('[data-testid^="viz-row-"]').allInnerTexts()).map((t) => /(Alpha|Beta|Gamma)/.exec(t)?.[1] ?? "").filter(Boolean);
  await expect.poll(order).toEqual(["Alpha", "Beta", "Gamma"]);
  await rowOf(page, "Gamma item").dragTo(rowOf(page, "Alpha item"), { sourcePosition: { x: 4, y: 10 }, targetPosition: { x: 4, y: 2 } });
  await expect.poll(order).toEqual(["Gamma", "Alpha", "Beta"]);
});
