import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A `role: view` param is state the page or the system sets: it rides the
// `#` like a filter but the rail never draws it. Definition's `params` is
// one, so its rail has no Filters group.

const numbat = `title: Viewed Numbat
category: Data
loaders:
  rows:
    sql: SELECT 'Numbat' AS line
view:
  viz: table
  from: rows
`;

test("Definition's rail has no Filters group: its params param is a view param", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-viewrole", { files: { "pages/numbat.yaml": numbat } });
  await page.goto("/");
  await openFromLauncher(page, "Viewed Numbat");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-about").click();
  await page.getByTestId("page-about-definition").click();

  const file = page.getByTestId("page-file").filter({ visible: true });
  await expect(file.getByTestId("node-about")).toContainText("Viewed Numbat");
  // The shown tab's rail: the New Session tab beside it keeps its own filters mounted.
  await expect(page.getByTestId("page-rail-filters").filter({ visible: true })).toHaveCount(0);
  await expect(page.getByTestId("page-param-params").filter({ visible: true })).toHaveCount(0);
});
