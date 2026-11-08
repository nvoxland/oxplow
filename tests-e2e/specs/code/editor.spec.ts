import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown, load } from "../../support/ui.js";

// The editor is oxplow-foundation's page over the `editor` viz: a file of
// the working tree, its unsaved edits marked on its tab (●), saved with
// Ctrl/Cmd+S through `oxplow.file.save`, and a dirty tab closed with Undo.

const TAB = "oxplow:oxplow/file?path=emu.txt";

// A page of the suite's own that links to the file, as any list does.
const list = `title: List Emu
category: Data
loaders:
  files:
    sql: SELECT 'emu.txt' AS name, 'oxplow:oxplow/file?path=emu.txt' AS target
view:
  from: files
  viz: table
  columns:
  - key: name
    label: Name
    link: { kind: page, from: target }
`;

async function openEmu(page: import("@playwright/test").Page, daemon: Parameters<typeof installExtension>[0]) {
  await installExtension(daemon, "e2e-emu", { files: { "pages/emu.yaml": list } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_emu/emu");
  await shown(page).getByTestId("page-file").getByRole("button", { name: "emu.txt" }).filter({ visible: true }).first().click();
  const editor = shown(page).getByTestId("viz-editor").locator(".monaco-editor").first();
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("emu");
  return editor;
}

test("typing marks the tab dirty; Ctrl/Cmd+S writes the file and clears it", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await load(page);
  const editor = await openEmu(page, daemon);
  // The caret at the start of the first line.
  await editor.locator(".view-line").first().click();
  await page.keyboard.press("Home");
  await page.keyboard.type("kiwi ");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).not.toBeVisible();
  expect(readFileSync(join(daemon.project, "emu.txt"), "utf8")).toBe("kiwi emu\n");
});

test("closing a dirty tab offers Undo, which brings the tab and its draft back", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "emu.txt"), "emu\n");
  await load(page);
  const editor = await openEmu(page, daemon);
  // The caret at the start of the first line.
  await editor.locator(".view-line").first().click();
  await page.keyboard.press("Home");
  await page.keyboard.type("owl ");
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await page.getByTestId(`center-tab-close-${TAB}`).click();
  await expect(page.getByTestId(`center-tab-${TAB}`)).not.toBeVisible();
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(`center-tab-dirty-${TAB}`)).toBeVisible();
  await expect(shown(page).getByTestId("viz-editor").locator(".monaco-editor").first()).toContainText("owl emu");
  expect(readFileSync(join(daemon.project, "emu.txt"), "utf8")).toBe("emu\n");
});
