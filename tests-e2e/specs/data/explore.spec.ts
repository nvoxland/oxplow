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

// Explore Data is oxplow-foundation's custom page: the catalog is its rail
// (a lens), the SQL box its component, and the result oxplow draws under
// it from the spec the component hands it, with Save as Lens beside it.
test("explore a model, run SQL, and keep it as a lens through lens.keep", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Echidna task", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Explore Data");
  await expect(page.getByTestId("page-custom")).toBeVisible();
  await page.getByTestId("custom-page-rail").getByText("v_work_item", { exact: true }).click();
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  // Picking a model writes its query; replace it once it's there.
  await expect(frame.getByTestId("explore-sql")).toHaveValue(/v_work_item/);
  await frame.getByTestId("explore-sql").fill("SELECT title FROM v_work_item WHERE title LIKE 'Echidna%'");
  await frame.getByTestId("explore-run").click();
  await expect(page.getByTestId("custom-page-spec").getByTestId("lens-row-0")).toContainText("Echidna task");
  const before = await keeps(daemon);
  await page.getByTestId("explore-save-open").click();
  await page.getByTestId("explore-save-title").fill("Echidna tasks");
  await page.getByTestId("explore-save").click();
  await expect(page.locator('[data-testid="page-lens"]:visible').getByTestId("lens-row-0")).toContainText("Echidna task");
  // The save was one `oxplow.lens.keep`, whatever other specs kept before it.
  expect(await keeps(daemon)).toBe(before + 1);
});

// A refused query is the component's to show: oxplow answers it with the
// reason, and draws nothing.
test("a query oxplow refuses says why in the explorer", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Explore Data");
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  await frame.getByTestId("explore-sql").fill("SELECT * FROM no_such_model");
  await frame.getByTestId("explore-run").click();
  await expect(frame.getByTestId("explore-error")).toBeVisible();
  await expect(page.getByTestId("custom-page-spec")).toHaveCount(0);
});
