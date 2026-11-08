import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { inFrame, open, shown } from "../../support/ui.js";

// Only what the shell shows: the Integrations frame's config form (oxplow's
// `openForm`), check, toggle and sync, and the right-click choice of the work
// list. That an item reaches the tracker and comes back on a sync is the
// provider tests' (`providers/tests.rs`).
test("a person configures, checks and enables the fake tracker from Integrations", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings#area=integrations");
  // Integrations is oxplow-foundation's own component, in a frame; the
  // config form, like every form it opens, is oxplow's (`openForm`).
  let integrations = await inFrame(page, "Integrations");
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
  // An item on the active work list, so the frame's sync has one to bring.
  await run(daemon, "oxplow.work_item.create", { title: "Kiwi from oxplow" });
  // An external item reaches oxplow's models through its collector's read.
  // Integrations is the last section of a long page, and the choices above
  // changed the Capabilities table over it: the frame may now be below the
  // fold, where the browser doesn't run its animation frames, and a click
  // waits for the button to be stable by those. A person scrolls to it.
  integrations = await inFrame(page, "Integrations");
  await integrations.getByTestId("integration-sync-e2e/fake-work_items").click();
  await expect(integrations.getByTestId("integration-collector-e2e/fake-work_items")).toContainText(/1 record/);
});

// A command the frame runs that fails says so once. One that names a field
// (`INVALID` at `/config/team`) is the component's to show, in its row; any
// other is oxplow's, under the frame and recorded (pages/action-failure.spec.ts).
test("a refused enable in Integrations names its field once, in the row", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings#area=integrations");
  const integrations = await inFrame(page, "Integrations");
  await expect(integrations.getByTestId("integration-status-e2e/fake")).toContainText("Off");
  // Its config needs a team: enabling it unconfigured is refused.
  await integrations.getByTestId("integration-toggle-e2e/fake").click();
  await expect(integrations.getByTestId("integration-note-e2e/fake")).toContainText("a team is required");
  await expect(shown(page).getByTestId("custom-component-refused")).not.toBeVisible();
  const recorded = async () => (await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_op_error" })).rows[0]?.[0];
  expect(await recorded()).toBe(0);
});
