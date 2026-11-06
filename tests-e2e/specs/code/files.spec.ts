import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Which file New File… makes is the foundation's fixture's; that the app
// opens it in the Editor (the command's open_after) is only seen here.
test("New File… from a file's menu opens the new file in the Editor", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  await page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "emu.txt" }).filter({ visible: true }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-new-file").click();
  const field = page.getByTestId("viz-action-form-fields-path");
  await field.fill("kiwi.txt");
  await field.press("Enter");
  await expect(page.locator(".monaco-editor").first()).toBeVisible();
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
  await expect(page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "..", exact: true })).toHaveCount(0);
});
