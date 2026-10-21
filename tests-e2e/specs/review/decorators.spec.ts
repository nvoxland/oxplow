import { ipc, run, searchable, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

// oxplow-bundled decorates an effort with its latest verdict: a chip
// in the effort page's header.

test("an accepted effort's page carries the review's verdict chip", async ({ page, daemon }) => {
  const created = await run<{ result: { ref: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat fix",
    state: "in_progress",
    thread: daemon.threadUri,
  });
  // The effort opens in the background once the item is in progress.
  let effort = "";
  await until("the item's effort to open", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT ref FROM v_effort WHERE work_item = '${created.result.ref}'`,
    });
    effort = (out.rows[0]?.[0] as string | undefined) ?? "";
    return effort !== "";
  });
  expect(effort).toMatch(/^oxplow:oxplow\/effort\?id=eff\d+$/);
  await run(daemon, "oxplow.review.accept", { ref: effort });
  await searchable(daemon, "Wombat fix", "work_item");
  await page.goto("/");
  await searchLauncher(page, "Wombat fix");
  await page.getByTestId(`launcher-hit-${created.result.ref}`).click();
  // The task's effort, its Review: the effort's own page.
  await page.locator('[data-testid^="tasks-show-in-history-"]').first().click();
  await expect(page.getByTestId("page-chips")).toContainText("Accepted");
  // It leads with oxplow-bundled's verdict: no tests ran in this effort.
  await expect(page.getByTestId("replacement-effort.verdict")).toContainText("Tests: none ran");
});
