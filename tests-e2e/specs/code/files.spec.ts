import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Files page is the foundation's page: a folder at a time, with a file's
// actions in its right-click menu (a one-field form is a prompt strip).
test("the Files page lists the project and renames a file from its menu", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  const row = page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "emu.txt" }).filter({ visible: true });
  await expect(row).toBeVisible();
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-rename").click();
  const field = page.getByTestId("viz-action-form-fields-to");
  await field.fill("owl.txt");
  await field.press("Enter");
  await expect(page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "owl.txt" }).filter({ visible: true })).toBeVisible();
  expect(existsSync(join(daemon.project, "owl.txt"))).toBe(true);
  expect(existsSync(join(daemon.project, "emu.txt"))).toBe(false);
});

// New File… creates the file and opens it, as the command's open_after says.
test("New File… from a file's menu creates the file and opens it", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  await page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "emu.txt" }).filter({ visible: true }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-new-file").click();
  const field = page.getByTestId("viz-action-form-fields-path");
  await field.fill("kiwi.txt");
  await field.press("Enter");
  await expect(page.locator(".monaco-editor").first()).toBeVisible();
  expect(existsSync(join(daemon.project, "kiwi.txt"))).toBe(true);
});

// A folder opens in place: its children are read when it opens, nested
// under it, and the page keeps no `..` row.
test("a folder on the Files page opens in place and lists its children", async ({ page, daemon }) => {
  mkdirSync(join(daemon.project, "sub"));
  writeFileSync(join(daemon.project, "sub", "inner.txt"), "inner\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  await expect(page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "inner.txt" })).toHaveCount(0);
  await page
    .locator('[data-testid="viz-tree-row"]:visible')
    .filter({ hasText: "sub" })
    .getByRole("button", { name: "Expand" })
    .click();
  await expect(page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "inner.txt" }).filter({ visible: true })).toBeVisible();
  await expect(page.getByRole("button", { name: ".." })).toHaveCount(0);
});
