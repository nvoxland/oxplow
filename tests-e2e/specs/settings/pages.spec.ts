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

// An area is Settings at another view: one page, re-rendered, its area in
// the tab's address, and a step Back returns from.
test("a Settings area is the page's view: the same page, and Back returns to the one before", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  const body = page.locator('[data-testid^="center-tab-body-page:settings"]:visible');
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-page:settings");
  // A mark on the page's own element (the rail inside it may redraw as
  // its layout settles; the page must not mount again).
  await body.getByTestId("page-settings").evaluate((el) => {
    el.setAttribute("data-mounted-once", "yes");
  });
  await body.getByTestId("settings-area-data").click();
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-page:settings?area=data");
  await expect(body.getByTestId("page-settings")).toHaveAttribute("data-mounted-once", "yes");
  await page.getByTestId("page-nav-back").filter({ visible: true }).click();
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-page:settings");
  await expect(body.getByRole("heading", { level: 1 })).toHaveText("Settings");
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

// Stream Settings is the foundation's document over `v_stream`: its prompt
// is an edit form (Save only when changed), saved through the command, and
// kept across a reload.
test("a stream's prompt is edited and saved on its Settings page", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-testid^="navigator-strip-stream-"]').first().click({ button: "right" });
  await page.getByTestId("menu-item-stream.settings").click();
  const prompt = page.locator('[data-testid="lens-part-prompt"]:visible');
  const box = prompt.getByRole("textbox");
  const save = prompt.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await box.fill("Always answer in rhyme.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(page.locator('[data-testid="lens-part-prompt"]:visible').getByRole("textbox")).toHaveValue("Always answer in rhyme.");
});
