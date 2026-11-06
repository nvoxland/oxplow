import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Changes page is the foundation's `changes` document, the renderer of
// an effort's (and a snapshot's, a turn's) page. What its parts say, and what
// Rename… does, are the foundation's fixtures'; the app shows the effort's
// page opened from its URI and the picker turning it into a range.
test("an effort's page opens from its URI, and the pickers turn it into a range", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Quail fix",
    state: "in_progress",
    thread: daemon.thread,
  });
  let effortUri = "";
  await until("the item's effort to open", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT uri FROM v_effort WHERE work_item = '${created.result.uri}'`,
    });
    effortUri = (out.rows[0]?.[0] as string | undefined) ?? "";
    return effortUri !== "";
  });
  await page.goto("/");
  // The window answers a navigate once its stream is shown: a page open is the sign.
  await openFromLauncher(page, "Suite Home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  await run(daemon, "oxplow.window.navigate", { uri: effortUri });

  const effort = page.locator('[data-node="effort"]');
  await expect(effort).toContainText("Open");
  await expect(page.locator('[data-node="header"]')).toContainText("working tree");
  // Picking an end leaves the effort for a plain comparison.
  await page.getByTestId("page-param-end-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-end-edit").filter({ visible: true }).selectOption("working");
  // The effort part stays a node of the page, showing no effort and none of
  // its actions (they act on the effort's row).
  await expect(effort.getByRole("button")).toHaveCount(0);
  await expect(page.locator('[data-node="header"]')).toContainText("Comparison");
});
