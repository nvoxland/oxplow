import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { shown, load } from "../../support/ui.js";

// The test extension's effect deletes a task titled "[delete]": a
// destructive step oxplow never runs for an effect, so it waits as a
// proposal for a person.

test("an effect's destructive step waits in Alerts until a person approves it", async ({ page, daemon }) => {
  await load(page);
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
  await expect(shown(page).getByTestId(`proposal-${id}`)).toHaveCount(0);
  await expect.poll(live).toBe(0);
});

test("approving a proposal whose run fails keeps the failure on its card, recorded once", async ({ page, daemon }) => {
  await load(page);
  const made = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", { title: "Quince [delete]", thread: daemon.thread });
  await expect(page.getByTestId("dock-alerts-badge")).toHaveText("1");
  // The task goes another way first, so the delete the effect asked for fails.
  await run(daemon, "oxplow.work_item.delete", { uri: made.result.uri });
  await page.getByTestId("dock-alerts").click();
  await page.getByTestId("dock-panel-open-page").getByTestId("open-page-link").click();
  const card = shown(page).locator('[data-testid^="proposal-"]').filter({ hasText: "destructive" }).first();
  const id = (await card.getAttribute("data-testid"))!.replace("proposal-", "");
  await page.getByTestId(`proposal-approve-${id}-trigger`).click();
  await page.getByTestId(`proposal-approve-${id}-confirm`).click();
  // The proposal stays pending, and the list re-reads: the card drawn again
  // still says why.
  await expect(shown(page).getByTestId(`proposal-error-${id}`)).toBeVisible();
  const failed = async () =>
    (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_op_error" })).rows[0]?.[0];
  await expect.poll(failed).toBe(1);
  // The record links the proposal and the audit row of the run that failed.
  const linked = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT o.proposal_id, a.outcome FROM v_op_error o JOIN v_command_audit a ON a.id = o.audit_id",
  });
  expect(linked.rows).toEqual([[Number(id), "error"]]);
});
