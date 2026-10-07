import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/** How many times `oxplow.lens.keep` has run on `daemon`. */
async function keeps(daemon: Parameters<typeof ipc>[0]): Promise<number> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT 1 FROM v_event WHERE type = 'command.executed' AND json_extract(payload, '$.command') = 'oxplow.lens.keep'",
  });
  return out.rows.length;
}

test("explore a model, run SQL, and keep it as a lens through lens.keep", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Echidna task", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Explore Data");
  await page.getByTestId("explore-entity-v_task").click();
  // Picking an entity writes its query; replace it once it's there.
  await expect(page.getByTestId("explore-sql")).toHaveValue(/v_task/);
  await page.getByTestId("explore-sql").fill("SELECT title FROM v_task WHERE title LIKE 'Echidna%'");
  await page.getByTestId("explore-run").click();
  await expect(page.getByTestId("page-explore-data").getByTestId("lens-row-0")).toContainText("Echidna task");
  const before = await keeps(daemon);
  await page.getByTestId("explore-save-open").click();
  await page.getByTestId("explore-save-title").fill("Echidna tasks");
  await page.getByTestId("explore-save").click();
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Echidna task");
  // The save was one `oxplow.lens.keep`, whatever other specs kept before it.
  expect(await keeps(daemon)).toBe(before + 1);
});
