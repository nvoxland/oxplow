import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

// The test extension's effect deletes a task titled "[delete]": a
// destructive step oxplow never runs for an effect, so it waits as a
// proposal for a person.

test("an effect's destructive step waits in Alerts until a person approves it", async ({ page, daemon }) => {
  await page.goto("/");
  await run(daemon, "oxplow.work_item.create", { title: "Pear [delete]", thread: daemon.thread });
  // The dock's Alerts icon counts it once the effect has run (it runs in the
  // background, so wait for that). It opens the Alerts panel, which links
  // to the Alerts page, where the proposal waits.
  const bell = page.getByTestId("dock-alerts");
  await expect(page.getByTestId("dock-alerts-badge")).toHaveText("1");
  await bell.click();
  await page.getByTestId("dock-panel-open-page").getByTestId("open-page-link").click();
  const card = page.locator('[data-testid^="proposal-"]').filter({ hasText: "destructive" }).first();
  await expect(card).toBeVisible();
  const id = (await card.getAttribute("data-testid"))!.replace("proposal-", "");
  const live = async () =>
    (
      await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT count(*) FROM v_work_item WHERE title = 'Pear [delete]'",
      })
    ).rows[0]?.[0];
  expect(await live()).toBe(1);
  await page.getByTestId(`proposal-approve-${id}-trigger`).click();
  await page.getByTestId(`proposal-approve-${id}-confirm`).click();
  await expect(page.getByTestId(`proposal-${id}`)).toHaveCount(0);
  await expect.poll(live).toBe(0);
});
