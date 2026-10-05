import { approveProgram, ipc, run, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The test extension's effect comments on each created task; a title with
// "[fail]" makes it comment on an item that doesn't exist.

const EFFECT = "effect:e2e/note-created";

async function sql(daemon: Daemon, query: string): Promise<unknown[][]> {
  return (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: query })).rows;
}

/** The item titled `title`'s ref. */
async function refOf(daemon: Daemon, title: string): Promise<string> {
  const rows = await sql(daemon, `SELECT ref FROM v_work_item WHERE title = '${title}'`);
  return String(rows[0]?.[0]);
}

/** The effect's comments on the task titled `title`. */
async function notesOn(daemon: Daemon, title: string): Promise<number> {
  const rows = await sql(
    daemon,
    `SELECT count(*) FROM v_task_note n JOIN v_task t ON t.id = n.task_id
      WHERE t.title = '${title}' AND n.body = 'Noted by the suite''s effect.'`,
  );
  return Number(rows[0]?.[0]);
}

test("a failed reaction is in Delivery, and a person's Retry composes it afresh", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await approveProgram(daemon, "effect", "e2e/note-created");
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  await run(daemon, "work_item.create", { title: "Plum [fail]", thread: daemon.thread });
  const row = page.locator('[data-testid^="reaction-row-e2e/note-created-"]');
  await expect(row).toBeVisible();
  await expect(row).toContainText("tsk999999");
  // The person fixes what made it fail, then retries: the effect reads the
  // item as it is now.
  await run(daemon, "work_item.update", { ref: await refOf(daemon, "Plum [fail]"), title: "Plum" });
  const key = (await row.getAttribute("data-testid"))!.replace("reaction-row-", "");
  await page.getByTestId(`reaction-retry-${key}-trigger`).click();
  await page.getByTestId(`reaction-retry-${key}-confirm`).click();
  await expect(row).toHaveCount(0);
  await expect.poll(() => notesOn(daemon, "Plum")).toBe(1);
});

test("a person's Backfill reaches what was logged before the effect was approved", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "work_item.create", { title: "Fig", thread: daemon.thread });
  await approveProgram(daemon, "effect", "e2e/note-created");
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  await page.getByTestId(`effect-backfill-${EFFECT}`).click();
  await expect(page.getByTestId(`effect-backfill-ask-${EFFECT}`)).toBeVisible();
  await page.getByTestId(`effect-backfill-run-${EFFECT}`).click();
  await expect.poll(() => notesOn(daemon, "Fig")).toBe(1);
});
