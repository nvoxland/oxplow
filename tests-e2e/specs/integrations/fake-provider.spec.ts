import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { approveProgram, ipc, run, until } from "../../support/daemon.js";
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
  await approveProgram(daemon, "provider", "e2e/fake");
  await page.goto("/");
  await openFromLauncher(page, "Settings");
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
  await page.getByTestId("pieces-work_items-project-fake").click();
  await expect(page.getByTestId("pieces-work_items-project-fake")).toBeChecked();
  // Made with no provider named: the active one, which Pieces chose.
  await run(daemon, "work_item.create", { title: "Kiwi from oxplow" });
  await expect.poll(() => fakeState(daemon.project)).toContain("Kiwi from oxplow");
  // An external item reaches oxplow's models through its collector's read.
  await page.getByTestId("integration-sync-e2e/fake-work_items").click();
  await expect(page.getByTestId("integration-collector-e2e/fake-work_items")).toContainText(/1 record/);
  await openFromLauncher(page, "Board");
  // A tracker's item is on no thread.
  await page.getByTestId("board-scope").selectOption("all");
  await expect(page.getByTestId("work-board")).toContainText("Kiwi from oxplow");
  // The New Task page files on the active tracker too, on this thread
  // (tsk1059): it offers what any tracker takes, not oxplow's priority.
  await openNewTask(page);
  await expect(page.getByTestId("tasks-status")).toBeVisible();
  await expect(page.getByTestId("tasks-priority")).toHaveCount(0);
  await page.getByTestId("tasks-title").fill("Kiwi from the page");
  await page.keyboard.press("Enter");
  await expect.poll(() => fakeState(daemon.project)).toContain("Kiwi from the page");
  await run(daemon, "provider.sync", { instance: "e2e/fake", collector: "work_items" });
  await openFromLauncher(page, "Board");
  await page.getByTestId("board-scope").selectOption("thread");
  await expect(page.getByTestId("work-board")).toContainText("Kiwi from the page");
});
