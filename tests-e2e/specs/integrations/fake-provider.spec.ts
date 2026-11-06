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

test("a person configures, checks and enables the fake tracker, and an item reaches it", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings: Integrations");
  // Integrations is oxplow-foundation's own component, in a frame; the
  // config form, like every form it opens, is oxplow's (`openForm`).
  const integrations = page.frameLocator('iframe[aria-label="Integrations"]');
  await expect(integrations.getByTestId("integration-status-e2e/fake")).toContainText("Off");
  await integrations.getByTestId("integration-configure-e2e/fake").click();
  await page.getByTestId("custom-page-own-form-fields-team").fill("core");
  await page.getByTestId("custom-page-own-form-fields-submit").click();
  // Check starts it with this config, asks it, and shows what it said —
  // before anything is saved or enabled.
  await expect(integrations.getByTestId("integration-status-e2e/fake")).toContainText("Off");
  await integrations.getByTestId("integration-check-e2e/fake").click();
  await expect(integrations.getByTestId("integration-status-e2e/fake")).toContainText("Ready");
  await integrations.getByTestId("integration-toggle-e2e/fake").click();
  await until("the fake to be enabled and ready", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: "SELECT 1 FROM v_provider_instance WHERE instance = 'e2e/fake' AND enabled = 1 AND state = 'ready'",
    });
    return out.rows.length === 1;
  });
  await expect(integrations.getByTestId("integration-status-e2e/fake")).toContainText("Ready");
  const fakeRow = page.locator("tr", { hasText: "fake" }).filter({ hasText: /Fake|fake tracker/i }).first();
  await fakeRow.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-use-project").click();
  await expect(fakeRow).toContainText("The project uses it");
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
  // Integrations is the last section of a long page, and the choices above
  // changed the Capabilities table over it: the frame may now be below the
  // fold, where the browser doesn't run its animation frames, and a click
  // waits for the button to be stable by those. A person scrolls to it.
  await page.locator('iframe[aria-label="Integrations"]').scrollIntoViewIfNeeded();
  await integrations.getByTestId("integration-sync-e2e/fake-work_items").click();
  await expect(integrations.getByTestId("integration-collector-e2e/fake-work_items")).toContainText(/1 record/);
  await openFromLauncher(page, "Board");
  // A tracker's item is on no thread.
  await page.getByTestId("page-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-show-edit").filter({ visible: true }).selectOption("all");
  await expect(page.getByTestId("viz-board")).toContainText("Kiwi from oxplow");
  // The New Task page files on the active tracker too, on this thread
  // (tsk1059): it offers what any tracker takes, not oxplow's priority.
  await openNewTask(page);
  await expect(page.getByTestId("viz-form-fields-state")).toBeVisible();
  await expect(page.getByTestId("viz-form-fields-native.priority")).toHaveCount(0);
  await page.getByTestId("viz-form-fields-title").fill("Kiwi from the page");
  await page.keyboard.press("Enter");
  await expect.poll(() => fakeState(daemon.project)).toContain("Kiwi from the page");
  await run(daemon, "oxplow.provider.sync", { instance: "e2e/fake", collector: "work_items" });
  // The Board at its default view is another page beside the filtered one.
  await openFromLauncher(page, "Board");
  await page.getByTestId("page-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-show-edit").filter({ visible: true }).selectOption("thread");
  await expect(page.getByTestId("viz-board").filter({ visible: true })).toContainText("Kiwi from the page");
});
