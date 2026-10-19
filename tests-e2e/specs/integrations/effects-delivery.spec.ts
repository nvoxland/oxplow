import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { deploy, extensionSource, ipc, run, type Daemon } from "../../support/daemon.js";
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
    `SELECT count(*) FROM v_work_item_comment c JOIN v_work_item w ON w.ref = c.ref
      WHERE w.title = '${title}' AND c.body = 'Noted by the suite''s effect.'`,
  );
  return Number(rows[0]?.[0]);
}

test("a failed reaction is in Delivery, and a person's Retry composes it afresh", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Settings: Data");
  await run(daemon, "oxplow.work_item.create", { title: "Plum [fail]", thread: daemon.thread });
  const row = page.locator('[data-testid^="reaction-row-e2e/note-created-"]');
  await expect(row).toBeVisible();
  await expect(row).toContainText("tsk999999");
  // The person fixes what made it fail, then retries: the effect reads the
  // item as it is now.
  await run(daemon, "oxplow.work_item.update", { ref: await refOf(daemon, "Plum [fail]"), title: "Plum" });
  const key = (await row.getAttribute("data-testid"))!.replace("reaction-row-", "");
  await page.getByTestId(`reaction-retry-${key}-trigger`).click();
  await page.getByTestId(`reaction-retry-${key}-confirm`).click();
  await expect(row).toHaveCount(0);
  await expect.poll(() => notesOn(daemon, "Plum")).toBe(1);
});

test("a person's Backfill reaches what was logged before the effect was deployed", async ({ fresh }) => {
  const { page, daemon } = fresh;
  // Logged first; then an extension whose effect comments on created
  // tasks is deployed: it starts after what was already logged.
  await run(daemon, "oxplow.work_item.create", { title: "Fig", thread: daemon.thread });
  const dir = extensionSource(daemon, "late");
  mkdirSync(join(dir, "effects"), { recursive: true });
  writeFileSync(
    join(dir, "extension.yaml"),
    "manifest: 2\nname: late\nintent:\n  purpose: An effect deployed after the event it backfills.\n" +
      "permissions: { commands: [oxplow.work_item.comment] }\n" +
      "effects:\n  - { id: note, summary: Notes a created task., on: [work_item.created], entry: effects/note.star }\n",
  );
  writeFileSync(
    join(dir, "effects", "note.star"),
    'def transform(x):\n    return {"commands": [{"name": "oxplow.work_item.comment", "input": {"ref": x["event"]["payload"]["work_item"], "body": "Noted late."}}]}\n',
  );
  await deploy(daemon, "late");
  const late = async () =>
    Number(
      (
        await sql(
          daemon,
          "SELECT count(*) FROM v_work_item_comment c JOIN v_work_item w ON w.ref = c.ref WHERE w.title = 'Fig' AND c.body = 'Noted late.'",
        )
      )[0]?.[0],
    );
  expect(await late()).toBe(0);
  await page.goto("/");
  await openFromLauncher(page, "Settings: Data");
  await page.getByTestId("effect-backfill-late/note").click();
  await expect(page.getByTestId("effect-backfill-ask-late/note")).toBeVisible();
  await page.getByTestId("effect-backfill-run-late/note").click();
  await expect.poll(late).toBe(1);
});
