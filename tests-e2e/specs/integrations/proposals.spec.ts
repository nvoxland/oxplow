import { approveProgram, ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection } from "../../support/ui.js";

// The test extension's effect deletes a task titled "[delete]": a
// destructive step oxplow never runs for an effect, so it waits as a
// proposal for a person.

test("an effect's destructive step waits in Alerts until a person approves it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await approveProgram(daemon, "effect", "e2e/note-created");
  await page.goto("/");
  await run(daemon, "work_item.create", { title: "Pear [delete]", thread: daemon.thread });
  await expandRailSection(page, "core:alerts");
  const card = page.locator('[data-testid^="proposal-"]').filter({ hasText: "destructive" }).first();
  await expect(card).toBeVisible();
  const id = (await card.getAttribute("data-testid"))!.replace("proposal-", "");
  const live = async () =>
    (
      await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT count(*) FROM v_task WHERE title = 'Pear [delete]'",
      })
    ).rows[0]?.[0];
  expect(await live()).toBe(1);
  await page.getByTestId(`proposal-approve-${id}-trigger`).click();
  await page.getByTestId(`proposal-approve-${id}-confirm`).click();
  await expect(page.getByTestId(`proposal-${id}`)).toHaveCount(0);
  await expect.poll(live).toBe(0);
});
