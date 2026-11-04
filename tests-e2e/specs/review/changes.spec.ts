import { ipc, run, searchable, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

// The Changes page is the foundation's `changes` document, the renderer of
// an effort's (and a snapshot's, a turn's) page: the effort's own title and
// link, the range, the files, and the From/To pickers.
test("an effort's page names its effort, renames it, and the pickers turn it into a range", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Quail fix",
    state: "in_progress",
    thread: daemon.thread,
  });
  await until("the item's effort to open", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT uri FROM v_effort WHERE work_item = '${created.result.uri}'`,
    });
    return out.rows.length > 0;
  });
  await searchable(daemon, "Quail fix", "work_item");
  await page.goto("/");
  await searchLauncher(page, "Quail fix");
  await page.getByTestId(`launcher-hit-${created.result.uri}`).click();
  await page.locator('[data-node="activity"]').locator("a, button").filter({ hasText: "Quail fix" }).first().click();

  const effort = page.locator('[data-node="effort"]');
  await expect(effort).toContainText("Open");
  await expect(page.locator('[data-node="header"]')).toContainText("working tree");
  // Nothing was described: the section stays and says so.
  await expect(page.locator('[data-node="description"]')).toContainText("No description");

  // Rename… asks for the title in a form.
  await effort.getByTestId("viz-action-rename").click();
  const form = page.getByTestId("viz-action-form");
  await form.getByTestId("viz-action-form-fields-title").fill("Quail fix, renamed");
  await form.getByTestId("viz-action-form-fields-submit").click();
  await expect(effort).toContainText("Quail fix, renamed");

  // Picking an end leaves the effort for a plain comparison.
  await page.getByTestId("page-param-end-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-end-edit").filter({ visible: true }).selectOption("working");
  // The effort part stays a node of the page, showing no effort and none of
  // its actions (they act on the effort's row): its title, over a line that
  // says there is none.
  await expect(effort).toHaveText("EffortNot part of an effort.");
  await expect(effort.getByRole("button")).toHaveCount(0);
  await expect(page.locator('[data-node="header"]')).toContainText("Comparison");
  await expect(page.locator('[data-node="header"]')).toContainText("working tree");
});
