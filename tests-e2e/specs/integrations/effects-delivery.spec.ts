import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { deploy, extensionSource, fileTask, ipc, run, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, taskIdOf } from "../../support/ui.js";

// The test extension's effect bookmarks each created task under its title;
// a title with "[fail]" makes it bookmark in a stream that doesn't exist.

const EFFECT = "oxplow:oxplow/effect?effect=e2e/note-created";

async function sql(daemon: Daemon, query: string): Promise<unknown[][]> {
  return (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: query })).rows;
}

/** The item titled `title`'s ref. */
async function refOf(daemon: Daemon, title: string): Promise<string> {
  const rows = await sql(daemon, `SELECT uri FROM v_oxplow_task WHERE title = '${title}'`);
  return String(rows[0]?.[0]);
}

/** The effect's bookmarks of the task titled `title`, under that title. */
async function notesOn(daemon: Daemon, title: string): Promise<number> {
  const rows = await sql(
    daemon,
    `SELECT count(*) FROM v_bookmark b JOIN v_oxplow_task w ON w.uri = b.uri
      WHERE w.title = '${title}' AND b.label = w.title`,
  );
  return Number(rows[0]?.[0]);
}

test("a failed reaction is in Delivery, and a person's Retry composes it afresh", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings#area=data");
  await fileTask(daemon, "Plum [fail]");
  const row = page.locator("tr", { hasText: "FOREIGN KEY constraint failed" });
  await expect(row).toBeVisible();
  // The person fixes what made it fail, then retries: the effect reads the
  // item as it is now.
  await run(daemon, "oxtask.task.update", { id: taskIdOf(await refOf(daemon, "Plum [fail]")), title: "Plum" });
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-retry").click();
  await page.getByTestId("viz-action-confirm-run").click();
  await expect(row).toHaveCount(0);
  await expect.poll(() => notesOn(daemon, "Plum")).toBe(1);
});

test("a person's Backfill reaches what was logged before the effect was deployed", async ({ page, daemon }) => {
  // Logged first; then an extension whose effect bookmarks created
  // tasks is deployed: it starts after what was already logged.
  await fileTask(daemon, "Fig");
  const dir = extensionSource(daemon, "late");
  mkdirSync(join(dir, "effects"), { recursive: true });
  writeFileSync(
    join(dir, "extension.yaml"),
    "manifest: 2\nname: late\nintent:\n  purpose: An effect deployed after the event it backfills.\n" +
      "requires: [oxtask]\npermissions: [sql.read, commands:oxplow.bookmark.set]\n" +
      "effects:\n  - { id: note, summary: Notes a created task., on: [oxplow.task_record.changed], where: { cause: trigger }, entry: effects/note.star, needs: [sql.read] }\n",
  );
  writeFileSync(
    join(dir, "effects", "note.star"),
    'def transform(x):\n    p = x["event"]["payload"]\n    if "state" not in p.get("changed", []) or "state" in (p.get("before") or {}):\n        return {"skip": "not a filing"}\n    rows = scope("sql.read", {"sql": "SELECT \'str\' || id AS stream FROM v_stream LIMIT 1", "params": {}})\n    return {"commands": [{"name": "oxplow.bookmark.set", "input": {"uri": p["uri"], "page_kind": "oxplow/task", "label": "Noted late.", "scope": "stream", "stream": rows[0]["stream"]}}]}\n',
  );
  await deploy(daemon, "late");
  const late = async () =>
    Number(
      (
        await sql(
          daemon,
          "SELECT count(*) FROM v_bookmark b JOIN v_oxplow_task w ON w.uri = b.uri WHERE w.title = 'Fig' AND b.label = 'Noted late.'",
        )
      )[0]?.[0],
    );
  expect(await late()).toBe(0);
  await open(page, daemon, "oxplow:oxplow/page?page=settings#area=data");
  // The effect's row opens on what a backfill would do; Backfill asks first.
  await page.locator("tr", { hasText: "late/note" }).getByRole("button", { name: "Expand" }).click();
  await expect(page.getByTestId("viz-action-backfill")).toBeVisible();
  await page.getByTestId("viz-action-backfill").click();
  await page.getByTestId("viz-action-confirm-run").click();
  await expect.poll(late).toBe(1);
});
