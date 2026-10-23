import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Files page is the foundation's lens: a folder at a time, with a file's
// actions in its right-click menu (a one-field form is a prompt strip).
test("the Files page lists the project and renames a file from its menu", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  const row = page.getByRole("button", { name: "emu.txt" }).filter({ visible: true });
  await expect(row).toBeVisible();
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-rename").click();
  const field = page.getByTestId("lens-action-form-fields-to");
  await field.fill("owl.txt");
  await field.press("Enter");
  await expect(page.getByRole("button", { name: "owl.txt" }).filter({ visible: true })).toBeVisible();
  expect(existsSync(join(daemon.project, "owl.txt"))).toBe(true);
  expect(existsSync(join(daemon.project, "emu.txt"))).toBe(false);
});

// New File… creates the file and opens it, as the command's open_after says.
test("New File… from a file's menu creates the file and opens it", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  await page.getByRole("button", { name: "emu.txt" }).filter({ visible: true }).click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-new-file").click();
  const field = page.getByTestId("lens-action-form-fields-path");
  await field.fill("kiwi.txt");
  await field.press("Enter");
  await expect(page.locator(".monaco-editor").first()).toBeVisible();
  expect(existsSync(join(daemon.project, "kiwi.txt"))).toBe(true);
});
