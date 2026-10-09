import { ipc, run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

// oxplow-bundled decorates an effort with its latest verdict: a chip
// in the effort page's header.

test("an accepted effort's page carries the review's verdict chip", async ({ page, daemon }) => {
  const created = await run<{ result: { ref: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat fix",
    state: "in_progress",
    thread: daemon.thread,
  });
  const effort = (
    await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT 'effort:eff' || id FROM v_effort WHERE work_item = '${created.result.ref}'`,
    })
  ).rows[0]?.[0] as string;
  expect(effort).toMatch(/^effort:eff\d+$/);
  await run(daemon, "oxplow.review.accept", { ref: effort });
  await searchable(daemon, "Wombat fix", "work_item");
  await page.goto("/");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("Wombat fix");
  await page.getByTestId(`launcher-hit-work_item:${created.result.ref.slice("work_item:".length)}`).click();
  // The task's effort, its Review: the effort's own page.
  await page.locator('[data-testid^="tasks-show-in-history-"]').first().click();
  await expect(page.getByTestId("page-chips")).toContainText("Accepted");
  // It leads with the verdict (tsk1036): no tests ran in this effort.
  await expect(page.getByTestId("effort-verdict-tests")).toContainText("Tests: none ran");
});
