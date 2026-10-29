import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Definition is a display mode of a page's YAML file: Definition in About this page opens
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

test("Definition in About this page opens the running copy drawn as Definition, and Open With lists the File viewer too", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat } });
  await page.goto("/");
  await openFromLauncher(page, "Defined Numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-about").click();
  await page.getByTestId("page-about-definition").click();

  const file = page.getByTestId("page-file").filter({ visible: true });
  // The tab names the file, not the page.
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  // The title is the nav bar's alone: the body draws no heading of its own.
  await expect(file.locator("h1")).toHaveCount(0);
  await expect(file.getByTestId("node-about")).toContainText("Defined Numbat");
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/numbat");
  await expect(file.getByTestId("node-description")).toContainText("What the numbats are.");
  // The raw YAML is the File viewer's mode, not a section here.
  await expect(file.getByTestId("node-yaml")).toHaveCount(0);
  // Its Source opens the same running copy, raw.
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/pages/numbat.yaml");
  await expect(file.getByTestId("node-modes")).toHaveCount(0);

  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await expect(page.getByTestId("menu-item-page-open-with-oxplow-foundation/definition")).toContainText("✓");
  await expect(page.getByTestId("menu-item-page-open-with-oxplow-foundation/file-viewer")).toBeVisible();
  await page.getByTestId("menu-item-page-open-with-oxplow-foundation/file-viewer").click();
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("title: Defined Numbat");
});

const other = `title: Other Numbat
category: Data
loaders:
  rows:
    sql: SELECT 'Other' AS line
view:
  viz: table
  from: rows
`;

test("Definition drawn again after another tab was shown still names its file and reads it", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat, "pages/other.yaml": other } });
  await page.goto("/");
  await openFromLauncher(page, "Defined Numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-about").click();
  await page.getByTestId("page-about-definition").click();
  const file = page.getByTestId("page-file").filter({ visible: true });
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");

  // Another tab is shown while the extension is deployed again (its running
  // copy is another deployment); coming back, the tab still names its file
  // and its sections read it.
  const tab = page.locator('[data-testid^="center-tab-"]').filter({ hasText: "Definition: numbat.yaml" }).first();
  await openFromLauncher(page, "Other Numbat");
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Other Numbat");
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat.replace("What the numbats are.", "What they were."), "pages/other.yaml": other } });
  await tab.click();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/numbat");
});
