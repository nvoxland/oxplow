import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

test("explore a model, run SQL, and keep it as a lens through lens.keep", async ({ page, daemon }) => {
  await run(daemon, "work_item.create", { title: "Echidna task", native: { thread: daemon.thread } });
  await page.goto("/");
  await openFromLauncher(page, "Explore Data");
  await page.getByTestId("explore-entity-v_task").click();
  // Picking an entity writes its query; replace it once it's there.
  await expect(page.getByTestId("explore-sql")).toHaveValue(/v_task/);
  await page.getByTestId("explore-sql").fill("SELECT title FROM v_task WHERE title LIKE 'Echidna%'");
  await page.getByTestId("explore-run").click();
  await expect(page.getByTestId("page-explore-data").getByTestId("lens-row-0")).toContainText("Echidna task");
  await page.getByTestId("explore-save-open").click();
  await page.getByTestId("explore-save-title").fill("Echidna tasks");
  await page.getByTestId("explore-save").click();
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Echidna task");
  const kept = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT json_extract(payload, '$.command') FROM v_event WHERE type = 'command.executed' AND json_extract(payload, '$.command') = 'lens.keep'",
  });
  expect(kept.rows.length).toBe(1);
});
