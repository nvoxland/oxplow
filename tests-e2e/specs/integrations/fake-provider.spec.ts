import { ipc, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { inFrame, open, shown } from "../../support/ui.js";

// Only what the shell shows: the Integrations frame's config form (oxplow's
// `openForm`), check and toggle. What a provider does once it runs is the provider
// tests' (`providers/tests.rs`).
test("a person configures, checks and enables the fake provider from Integrations", async ({ page, daemon }) => {
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
