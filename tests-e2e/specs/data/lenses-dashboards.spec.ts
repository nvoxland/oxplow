import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("a lens re-runs when its model changes, and pins to a new dashboard", async ({ page, daemon }) => {
  await run(daemon, "oxplow.lens.keep", {
    spec: { title: "Numbat tasks", query: "SELECT title FROM v_work_item WHERE title LIKE 'Numbat%' ORDER BY created_at", viz: "table" },
    extension: "mine",
    slug: "numbat-tasks",
  });
  await page.goto("/");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("Numbat tasks");
  await page.getByTestId("launcher-page-lens:mine/numbat-tasks").click();
  const lens = page.getByTestId("page-lens");
  await expect(lens).toContainText("No rows.");
  // A write elsewhere changes `v_work_item`: the open lens runs again by itself.
  await run(daemon, "oxplow.work_item.create", { title: "Numbat one", thread: daemon.thread });
  await expect(lens).toContainText("Numbat one");
  await page.getByTestId("lens-pin").click();
  await page.getByTestId("lens-pin-new").click();
  // It asks for the new dashboard's name (tsk1045); Enter creates it.
  await page.getByTestId("lens-pin-new-name").fill("Numbats");
  await page.keyboard.press("Enter");
  await expect
    .poll(async () => {
      const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT d.title, i.kind FROM v_dashboard_item i JOIN v_dashboard d ON d.id = i.dashboard_id WHERE json_extract(i.options_json, '$.lensId') = 'mine/numbat-tasks'",
      });
      return out.rows;
    })
    .toEqual([["Numbats", "lens"]]);
});
