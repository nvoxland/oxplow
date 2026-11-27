import { existsSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { fileTask, ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { shown, load, alertsBadge, openAlerts } from "../../support/ui.js";

// The test extension's effect deletes the project's file doomed.txt for a
// task titled "[delete]": a destructive step oxplow never runs for an effect,
// so it waits as a proposal for a person.

test("an effect's destructive step waits in Alerts until a person approves it", async ({ page, daemon }) => {
  await load(page);
  const doomed = join(daemon.project, "doomed.txt");
  writeFileSync(doomed, "soon gone\n");
  await fileTask(daemon, "Pear [delete]");
  // The dock's Alerts icon counts it once the effect has run (it runs in the
  // background, so wait for that). It opens the Alerts panel, which links
  // to the Alerts page, where the proposal waits.
  await expect(alertsBadge(page)).toHaveText("1");
  await openAlerts(page);
  const card = page.locator('[data-testid^="proposal-"]').filter({ hasText: "destructive" }).first();
  await expect(card).toBeVisible();
  const id = (await card.getAttribute("data-testid"))!.replace("proposal-", "");
  const live = () => (existsSync(doomed) ? 1 : 0);
  expect(live()).toBe(1);
  await page.getByTestId(`proposal-approve-${id}-trigger`).click();
  await page.getByTestId(`proposal-approve-${id}-confirm`).click();
  await expect(shown(page).getByTestId(`proposal-${id}`)).toHaveCount(0);
  await expect.poll(live).toBe(0);
});

test("approving a proposal whose run fails keeps the failure on its card, recorded once", async ({ page, daemon }) => {
  await load(page);
  await fileTask(daemon, "Quince [delete]");
  await expect(alertsBadge(page)).toHaveText("1");
  // The file was never there (or goes another way first), so the delete the
  // effect asked for fails.
  rmSync(join(daemon.project, "doomed.txt"), { force: true });
  await openAlerts(page);
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
  // The record links the proposal and the failed command run.
  const linked = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT o.proposal_id, a.outcome FROM v_op_error o JOIN v_command_run a ON a.id = o.command_run_id",
  });
  expect(linked.rows).toEqual([[Number(id), "failed"]]);
});
