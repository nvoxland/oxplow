import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The editor is oxplow-foundation's lens over the `editor` viz: a file of
// the working tree, its unsaved edits marked on its tab (●), saved with
// Ctrl/Cmd+S through `oxplow.file.save`, and a dirty tab closed with Undo.

const TAB = "oxplow:oxplow/file?path=emu.txt";

async function openEmu(page: import("@playwright/test").Page) {
  await openFromLauncher(page, "Files");
  await page.getByRole("button", { name: "emu.txt" }).filter({ visible: true }).first().click();
  const editor = page.getByTestId("lens-editor").filter({ visible: true }).locator(".monaco-editor").first();
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("emu");
  return editor;
}

test("typing marks the tab dirty; Ctrl/Cmd+S writes the file and clears it", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  const editor = await openEmu(page);
  // The caret at the start of the first line.
  await editor.locator(".view-line").first().click();
  await page.keyboard.press("Home");
  await page.keyboard.type("kiwi ");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toHaveCount(0);
  expect(readFileSync(join(daemon.project, "emu.txt"), "utf8")).toBe("kiwi emu\n");
});

test("closing a dirty tab offers Undo, which brings the tab and its draft back", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await page.goto("/");
  const editor = await openEmu(page);
  // The caret at the start of the first line.
  await editor.locator(".view-line").first().click();
  await page.keyboard.press("Home");
  await page.keyboard.type("owl ");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await page.getByTestId(`center-tab-close-${TAB}`).click();
  await expect(page.getByTestId(`center-tab-${TAB}`)).toHaveCount(0);
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await expect(page.getByTestId("lens-editor").filter({ visible: true }).locator(".monaco-editor").first()).toContainText("owl emu");
  expect(readFileSync(join(daemon.project, "emu.txt"), "utf8")).toBe("emu\n");
});
