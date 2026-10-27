import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/** How many times `oxplow.extension.write_page` has run on `daemon`. */
async function writes(daemon: Parameters<typeof ipc>[0]): Promise<number> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT 1 FROM v_event WHERE type = 'command.executed' AND json_extract(payload, '$.command') = 'oxplow.extension.write_page'",
  });
  return out.rows.length;
}

// Explore Data is oxplow-foundation's custom page: the catalog is its rail
// (a viz node picking the page's `model`), the SQL box its component, and
// the result oxplow draws under it from the inline page the component
// hands it, with Save as Page beside it. (A refused query's message is the
// foundation's `ui` fixture's.)
test("explore a model, run SQL, and save it as a page", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Echidna task", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Explore Data");
  await expect(page.locator('[data-testid="page-host"]:visible [data-testid="custom-component"]')).toBeVisible();
  await page.locator('[data-node="explore-catalog"]:visible').getByText("v_work_item", { exact: true }).click();
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  // Picking a model writes its query; replace it once it's there.
  await expect(frame.getByTestId("explore-sql")).toHaveValue(/v_work_item/);
  await frame.getByTestId("explore-sql").fill("SELECT title FROM v_work_item WHERE title LIKE 'Echidna%'");
  await frame.getByTestId("explore-run").click();
  await expect(page.getByTestId("custom-page-spec").getByTestId("viz-row-0")).toContainText("Echidna task");
  const before = await writes(daemon);
  await page.getByTestId("explore-save-open").click();
  await page.getByTestId("save-view-title").fill("Echidna tasks");
  await page.getByTestId("save-view-submit").click();
  // The saved page opens on its own, its rows as the view showed them.
  const saved = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?page=my-pages/echidna-tasks"]:visible');
  await expect(saved.getByTestId("viz-row-0")).toContainText("Echidna task");
  // The save was one `oxplow.extension.write_page`, whatever other specs wrote before it.
  expect(await writes(daemon)).toBe(before + 1);
});
