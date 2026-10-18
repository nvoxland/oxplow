import { readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Settings is a page per area, each listed in the rail every one shares.
const AREAS: [string, string][] = [
  ["agents", "Agents"],
  ["lsp", "Language Servers"],
  ["extensions", "Extensions"],
  ["data", "Data"],
  ["integrations", "Integrations"],
  ["ai", "AI"],
];

test("every Settings area renders, reached from the rail", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  const settings = page.locator('[data-testid="page-settings"]:visible');
  await expect(settings.getByRole("heading", { level: 1 })).toHaveText("Settings");
  await expect(settings.getByTestId("settings-search")).toBeVisible();
  for (const [id, title] of AREAS) {
    await settings.getByTestId(`settings-area-${id}`).click();
    await expect(page.locator('[data-testid="page-settings"]:visible').getByRole("heading", { level: 1 })).toHaveText(title);
  }
  await page.locator('[data-testid="settings-area-extensions"]:visible').click();
  await expect(page.locator('[data-testid="extensions-section"]:visible')).toBeVisible();
  await page.locator('[data-testid="settings-area-data"]:visible').click();
  await expect(page.locator('[data-testid="data-section"]:visible')).toBeVisible();
});

test("a config edit is saved to project.yaml and read back", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings: Agents");
  const field = page.getByTestId("settings-page-prompt-append");
  await field.fill("Prefer small, reviewable commits.");
  await page.locator('[data-testid="action-save"]:visible').click();
  await expect
    .poll(() => readFileSync(join(daemon.project, ".oxplow", "project.yaml"), "utf8"))
    .toContain("Prefer small, reviewable commits.");
  await page.reload();
  await openFromLauncher(page, "Settings: Agents");
  await expect(page.getByTestId("settings-page-prompt-append")).toHaveValue("Prefer small, reviewable commits.");
});
