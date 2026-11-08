import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// Definition is a display mode of a page's YAML file: Definition in About this page opens
// the page's running copy drawn as Definition, and the Text view (the raw
// YAML) is one View as away.

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

test("Definition in About this page opens the running copy drawn as Definition, and View as lists the Text view too", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-defn/numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-about").click();
  await page.getByTestId("page-about-definition").click();

  const file = shown(page).getByTestId("page-file");
  // The tab names the file, not the page.
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  // The title is the nav bar's alone: the body draws no heading of its own.
  await expect(file.locator("h1")).toHaveCount(0);
  await expect(file.getByTestId("node-about")).toContainText("Defined Numbat");
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/numbat");
  await expect(file.getByTestId("node-description")).toContainText("What the numbats are.");
  // The raw YAML is the Text view's mode, not a section here.
  await expect(file.getByTestId("node-yaml")).toHaveCount(0);
  // Its Source opens the same running copy, raw.
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/pages/numbat.yaml");
  await expect(file.getByTestId("node-modes")).toHaveCount(0);

  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await expect(page.getByTestId("menu-item-page-open-with-oxplow/definition")).toContainText("✓");
  await expect(page.getByTestId("menu-item-page-open-with-oxplow/file-viewer")).toBeVisible();
  await page.getByTestId("menu-item-page-open-with-oxplow/file-viewer").click();
  await expect(shown(page).getByTestId("viz-code")).toContainText("title: Defined Numbat");
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
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-defn/numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-about").click();
  await page.getByTestId("page-about-definition").click();
  const file = shown(page).getByTestId("page-file");
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");

  // Another tab is shown while the extension is deployed again (its running
  // copy is another deployment); coming back, the tab still names its file
  // and its sections read it.
  const tab = page.locator('[data-testid^="center-tab-"]').filter({ hasText: "Definition: numbat.yaml" }).first();
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-defn/other", { new_tab: true });
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Other Numbat");
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat.replace("What the numbats are.", "What they were."), "pages/other.yaml": other } });
  await tab.click();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  await expect(file.getByTestId("node-about")).toContainText("e2e-defn/numbat");
});

test("Definition of a page whose file is open in the Text view is another tab: both stay", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-defn", { files: { "pages/numbat.yaml": numbat } });
  await page.goto("/");
  const openDefinition = async () => {
    await open(page, daemon, "oxplow:oxplow/page?page=e2e-defn/numbat", { new_tab: true });
    await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
    await page.getByTestId("menu-item-action-page-about").click();
    await page.getByTestId("page-about-definition").click();
  };
  // Definition, then this tab to the Text view by View as: it switches in place.
  await openDefinition();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await page.getByTestId("menu-item-page-open-with-oxplow/file-viewer").click();
  await expect(shown(page).getByTestId("viz-code")).toContainText("title: Defined Numbat");
  // The page file's tabs, by id: the file's own (the display mode is the tab's state, not part of its id).
  const fileTabs = page.locator('[data-testid^="center-tab-oxplow:"][data-testid*="numbat.yaml"]');
  await expect(fileTabs).toHaveCount(1);
  await expect(fileTabs.first()).toHaveAttribute("data-testid", /center-tab-oxplow:oxplow\/file\?/);

  // The same page's Definition again: a Definition tab of its own, the Text view tab stays.
  await openDefinition();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Definition: numbat.yaml");
  await expect(fileTabs).toHaveCount(2);
  await expect(page.locator('[data-testid^="center-tab-oxplow:"][data-testid*="definition"]')).toHaveCount(1);
  await page.locator('[data-testid^="center-tab-oxplow:"][data-testid*="numbat.yaml"]:not([data-testid*="definition"])').click();
  await expect(shown(page).getByTestId("viz-code")).toContainText("title: Defined Numbat");
});
