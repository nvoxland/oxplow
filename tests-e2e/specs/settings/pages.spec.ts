import { readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const SECTIONS = [
  "Every Setting",
  "Agents",
  "Agent Prompt Additions",
  "Language Servers",
  "Extensions",
  "Data",
  "Integrations",
  "AI",
];

test("every Settings section renders", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  for (const title of SECTIONS) {
    await expect(page.getByRole("heading", { name: title, exact: true })).toBeVisible();
  }
  await expect(page.getByTestId("extensions-section")).toBeVisible();
  await expect(page.getByTestId("data-section")).toBeVisible();
});

test("a config edit is saved to project.yaml and read back", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  const field = page.getByTestId("settings-page-prompt-append");
  await field.fill("Prefer small, reviewable commits.");
  await page.getByTestId("settings-page-save").click();
  await expect
    .poll(() => readFileSync(join(daemon.project, ".oxplow", "project.yaml"), "utf8"))
    .toContain("Prefer small, reviewable commits.");
  await page.reload();
  await openFromLauncher(page, "Settings");
  await expect(page.getByTestId("settings-page-prompt-append")).toHaveValue("Prefer small, reviewable commits.");
});
