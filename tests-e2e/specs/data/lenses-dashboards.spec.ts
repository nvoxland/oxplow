import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("a lens re-runs when its model changes, and pins to a new dashboard", async ({ page, daemon }) => {
  await run(daemon, "lens.keep", {
    spec: { title: "Numbat tasks", query: "SELECT title FROM v_task WHERE title LIKE 'Numbat%' ORDER BY id", viz: "table" },
    extension: "mine",
    slug: "numbat-tasks",
  });
  await page.goto("/");
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("Numbat tasks");
  await page.getByTestId("launcher-page-lens:mine/numbat-tasks").click();
  const lens = page.getByTestId("page-lens");
  await expect(lens).toContainText("No rows.");
  // A write elsewhere changes `v_task`: the open lens runs again by itself.
  await run(daemon, "work_item.create", { title: "Numbat one", native: { thread: daemon.thread } });
  await expect(lens).toContainText("Numbat one");
  await page.getByTestId("lens-pin").click();
  await page.getByTestId("lens-pin-new").click();
  await expect
    .poll(async () => {
      const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT d.title, i.kind FROM v_dashboard_item i JOIN v_dashboard d ON d.id = i.dashboard_id",
      });
      return out.rows;
    })
    .toEqual([["My Dashboard", "lens"]]);
});
