import { ipc, run, until, waitForModels, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/** A one-value SQL read over the daemon's datasets. */
async function value(daemon: Daemon, sql: string, params: unknown[] = []): Promise<unknown> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql, params });
  return out.rows[0]?.[0] ?? null;
}

// Work handed from one thread to another (.context/agent-model.md
// "Handoffs and native messaging"): sent from the thread's own menu, seen on
// the other thread, taken there — and its item moves with it.
test("work handed off is seen on the other thread, and taking it moves its item", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const docs = await run<{ result: { id: string } }>(daemon, "oxplow.thread.create", {
    stream: daemon.stream,
    title: "Docs",
  });
  const docsId = docs.result.id;
  await waitForModels(daemon, ["v_work_item"], () =>
    run(daemon, "oxplow.work_item.create", { title: "Write the guide", thread: daemon.thread }),
  );
  await page.goto("/");

  // Hand Off Work… from the thread's menu: its form picks the thread and
  // the item from lists.
  await page.getByTestId(`navigator-strip-thread-${daemon.thread}`).click({ button: "right" });
  await page.getByTestId("menu-item-ref-commands-Threads").click();
  await page.getByTestId("menu-item-ref-command-oxplow.handoff.send").click();
  await page.getByTestId("viz-form-fields-to_thread").selectOption({ label: "Docs" });
  await page.getByTestId("viz-form-fields-work_item").selectOption({ label: "Write the guide" });
  await page.getByTestId("viz-form-fields-body").fill("The outline is done; write the rest.");
  await page.getByTestId("viz-form-fields-submit").click();
  await until("the handoff to be recorded", 15_000, async () => (await value(daemon, "SELECT count(*) FROM v_handoff")) === 1);

  // On the other thread its Tasks page shows it; Take moves the item there.
  await page.getByTestId(`navigator-strip-thread-${docsId}`).click();
  await openFromLauncher(page, "Tasks");
  const inbound = page.locator('[data-testid="page-file"]:visible [data-node="oxplow-foundation.thread-handoffs-in"]');
  await expect(inbound).toContainText("The outline is done");
  await inbound.getByText("The outline is done").click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-take").click();
  await until("the handoff to be taken", 15_000, async () => (await value(daemon, "SELECT state FROM v_handoff")) === "taken");
  // The work list restates its items after the move lands.
  await until(
    "the item to be on the other thread",
    15_000,
    async () => (await value(daemon, "SELECT 'thr' || thread_id FROM v_work_item WHERE title = ?1", ["Write the guide"])) === docsId,
  );
});
