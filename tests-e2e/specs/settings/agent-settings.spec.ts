import { readFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// Settings → Agents shows each enabled harness's own settings, as the
/// harness declares them: OpenCode's model once it's enabled, saved to
/// `agentConfig.opencode.model`. Nothing names a harness in the page.
test("an enabled harness's declared settings are offered and saved", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Settings: Agents");
  await expect(page.getByText("OpenCode — Model")).toHaveCount(0);
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "opencode"] });
  await page.reload();
  await openFromLauncher(page, "Settings: Agents");
  const row = page.getByRole("row", { name: /OpenCode — Model/ });
  await expect(row).toBeVisible();

  // Change… opens the command's form; its field starts from the saved value.
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-change").click();
  const form = page.getByTestId("viz-action-form");
  await form.getByTestId("viz-action-form-fields-value").fill("github-copilot/gpt-5");
  await form.getByTestId("viz-action-form-fields-submit").click();
  await expect
    .poll(() => readFileSync(join(daemon.project, ".oxplow", "project.yaml"), "utf8"))
    .toContain("github-copilot/gpt-5");
  await page.reload();
  await openFromLauncher(page, "Settings: Agents");
  await expect(page.getByRole("row", { name: /OpenCode — Model.*github-copilot\/gpt-5/ })).toBeVisible();
});
