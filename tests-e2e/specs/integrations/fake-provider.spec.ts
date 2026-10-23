import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, openNewTask } from "../../support/ui.js";

/** What the fake tracker's service holds (its state files), as text. */
function fakeState(project: string): string {
  const dir = join(project, ".oxplow");
  return readdirSync(dir)
    .filter((f) => f.startsWith("fake-state-"))
    .map((f) => readFileSync(join(dir, f), "utf8"))
    .join("\n");
}

test("a person configures, checks and enables the fake tracker, and an item reaches it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Settings: Integrations");
  await page.getByTestId("integration-config-e2e/fake-team").fill("core");
  // Check starts it with this config, asks it, and shows what it said —
  // before anything is saved or enabled.
  await expect(page.getByTestId("integration-status-e2e/fake")).toContainText("Off");
  await page.getByTestId("integration-check-e2e/fake").click();
  await expect(page.getByTestId("integration-status-e2e/fake")).toContainText("Ready");
  await page.getByTestId("integration-toggle-e2e/fake").click();
  await until("the fake to be enabled and ready", 30_000, async () => {
    const all = await ipc<Array<{ instance: string; enabled: boolean; health: { state: { state: string } } }>>(
      daemon,
      "list_provider_instances",
    );
    const fake = all.find((i) => i.instance === "e2e/fake");
    return !!fake && fake.enabled && fake.health.state.state === "ready";
  });
  await expect(page.getByTestId("integration-status-e2e/fake")).toContainText("Ready");
  await page.getByTestId("capabilities-work_items-project-fake").click();
  await expect(page.getByTestId("capabilities-work_items-project-fake")).toBeChecked();
  // The choice is the daemon's once it lists the fake as the active work list.
  await until("the fake to be the active work list", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: "SELECT 1 FROM v_capability_provider WHERE capability = 'work_items' AND provider = 'fake' AND active = 1 AND available = 1",
    });
    return out.rows.length === 1;
  });
  // Made with no provider named: the active one, which Settings → Capabilities chose.
  await run(daemon, "oxplow.work_item.create", { title: "Kiwi from oxplow" });
  await expect.poll(() => fakeState(daemon.project)).toContain("Kiwi from oxplow");
  // An external item reaches oxplow's models through its collector's read.
  await page.getByTestId("integration-sync-e2e/fake-work_items").click();
  await expect(page.getByTestId("integration-collector-e2e/fake-work_items")).toContainText(/1 record/);
  await openFromLauncher(page, "Board");
  // A tracker's item is on no thread.
  await page.getByTestId("lens-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("lens-param-show-edit").filter({ visible: true }).selectOption("all");
  await expect(page.getByTestId("lens-board")).toContainText("Kiwi from oxplow");
  // The New Task page files on the active tracker too, on this thread
  // (tsk1059): it offers what any tracker takes, not oxplow's priority.
  await openNewTask(page);
  await expect(page.getByTestId("lens-form-fields-state")).toBeVisible();
  await expect(page.getByTestId("lens-form-fields-native.priority")).toHaveCount(0);
  await page.getByTestId("lens-form-fields-title").fill("Kiwi from the page");
  await page.keyboard.press("Enter");
  await expect.poll(() => fakeState(daemon.project)).toContain("Kiwi from the page");
  await run(daemon, "oxplow.provider.sync", { instance: "e2e/fake", collector: "work_items" });
  // The Board at its default view is another page beside the filtered one.
  await openFromLauncher(page, "Board");
  await page.getByTestId("lens-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("lens-param-show-edit").filter({ visible: true }).selectOption("thread");
  await expect(page.getByTestId("lens-board").filter({ visible: true })).toContainText("Kiwi from the page");
});
