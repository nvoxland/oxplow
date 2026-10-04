import { ipc, run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

// Bundled oxplow-review decorates an effort with its latest verdict: a chip
// in the effort page's header.

test("an accepted effort's page carries the review's verdict chip", async ({ page, daemon }) => {
  const created = await run<{ result: { id: string } }>(daemon, "work_item.create", {
    title: "Wombat fix",
    state: "in_progress",
    native: { thread: daemon.thread },
  });
  const effort = (
    await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT 'effort:eff' || id FROM v_effort WHERE work_item = 'work_item:oxplow:${created.result.id}'`,
    })
  ).rows[0]?.[0] as string;
  expect(effort).toMatch(/^effort:eff\d+$/);
  await run(daemon, "oxplow_review.accept", { ref: effort });
  await searchable(daemon, "Wombat fix", "task");
  await page.goto("/");
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("Wombat fix");
  await page.getByTestId(`launcher-hit-task:${created.result.id}`).click();
  // The task's effort, its Details: the effort's own page.
  await page.locator('[data-testid^="tasks-show-in-history-"]').first().click();
  await expect(page.getByTestId("page-chips")).toContainText("Accepted");
});
