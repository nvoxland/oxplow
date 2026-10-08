import { readFileSync } from "node:fs";
import { join } from "node:path";

import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// Settings → Agents shows each enabled harness's own settings, as the
/// harness declares them: OpenCode's model once it's enabled, saved to
/// `agentConfig.opencode.model`. Nothing names a harness in the page.
test("an enabled harness's declared settings are offered and saved", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  await expect(page.getByTestId("settings-agent-opencode-model")).toHaveCount(0);
  await ipc(daemon, "set_agents", { agents: ["acp", "opencode"] });
  await page.reload();
  await openFromLauncher(page, "Settings");
  const model = page.getByTestId("settings-agent-opencode-model");
  await expect(model).toBeVisible();
  await model.fill("github-copilot/gpt-5");
  await page.getByTestId("settings-page-save").click();
  await expect
    .poll(() => readFileSync(join(daemon.project, ".oxplow", "project.yaml"), "utf8"))
    .toContain("github-copilot/gpt-5");
  await page.reload();
  await openFromLauncher(page, "Settings");
  await expect(page.getByTestId("settings-agent-opencode-model")).toHaveValue("github-copilot/gpt-5");
});
