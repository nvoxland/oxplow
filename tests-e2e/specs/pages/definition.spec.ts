import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Definition is a display mode of a page's YAML file: Show Definition opens
// the page's running copy drawn as Definition, and the File viewer (the raw
// YAML) is one Open With away.

const numbat = `title: Defined Numbat
description: What the numbats are.
category: Data
loaders:
  rows:
    sql: SELECT 'Numbat' AS line
view:
  viz: table
  from: rows
  columns: [{ key: line, label: Line }]
`;

test("Show Definition opens the running copy drawn as Definition, and Open With lists the File viewer too", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat } });
  await page.goto("/");
  await openFromLauncher(page, "Defined Numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-definition").click();

  const file = page.getByTestId("page-file").filter({ visible: true });
  // The tab names the file, not the page.
  await expect(file.locator("h1")).toHaveText("Definition: numbat.yaml");
  await expect(file.getByTestId("node-about")).toContainText("Defined Numbat");
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/numbat");
  await expect(file.getByTestId("node-description")).toContainText("What the numbats are.");
  // The raw YAML is the File viewer's mode, not a section here.
  await expect(file.getByTestId("node-yaml")).toHaveCount(0);
  await expect(file.getByTestId("node-modes")).toContainText("File Viewer");

  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await expect(page.getByTestId("menu-item-page-open-with-oxplow-foundation/definition")).toContainText("✓");
  await expect(page.getByTestId("menu-item-page-open-with-oxplow-foundation/file-viewer")).toBeVisible();
  await page.getByTestId("menu-item-page-open-with-oxplow-foundation/file-viewer").click();
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("title: Defined Numbat");
});
