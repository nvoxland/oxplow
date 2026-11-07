import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// A page re-runs by itself when a dataset it reads changes, and Pin to
// Dashboard makes a new dashboard page with it as an embed.
test("a page re-runs when its dataset changes, and pins to a new dashboard", async ({ page, daemon }) => {
  await run(daemon, "oxplow.extension.write_page", {
    extension: "mine",
    page: "numbat-tasks",
    spec: [
      "title: Numbat tasks",
      "category: Data",
      "loaders:",
      "  numbats:",
      "    sql: SELECT title FROM v_work_item WHERE title LIKE 'Numbat%' ORDER BY created_at",
      "view: { from: numbats, viz: table, empty: No numbats yet. }",
      "",
    ].join("\n"),
    deploy: true,
  });
  await open(page, daemon, "oxplow:oxplow/page?page=mine/numbat-tasks");
  const body = shown(page).getByTestId("page-file");
  await expect(body).toContainText("No numbats yet.");
  // A write elsewhere changes `v_work_item`: the open page runs again by itself.
  await run(daemon, "oxplow.work_item.create", { title: "Numbat one", thread: daemon.thread });
  await expect(body).toContainText("Numbat one");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-pin").click();
  await page.getByTestId("page-pin-new").click();
  // It asks for the new dashboard's name; Enter creates it.
  await page.getByTestId("page-pin-new-name").fill("Numbats");
  await page.keyboard.press("Enter");
  await expect(shown(page).getByTestId("dashboard-grid")).toContainText("Numbat one");
  await expect
    .poll(async () => {
      const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT id FROM v_page WHERE category = 'Dashboards' AND title = 'Numbats'" });
      const id = out.rows[0]?.[0];
      if (typeof id !== "string") return null;
      const dash = await ipc<{ view: { children: Array<{ kind: string; page: string }> } }>(daemon, "get_page", { id });
      return dash.view.children.map((c) => [c.kind, c.page]);
    })
    .toEqual([["embed", "mine/numbat-tasks"]]);
});
