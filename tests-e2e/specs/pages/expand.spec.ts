import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

const list = `title: Expand Wallaby
category: Data
loaders:
  animals:
    sql: SELECT 'wallaby' AS id, 'Wallaby' AS name UNION ALL SELECT 'bilby', 'Bilby' ORDER BY 1 DESC
view:
  id: animals
  viz: table
  from: animals
  columns:
  - { key: id, label: Id, expand: e2e_expand/wallaby-detail }
  - { key: name, label: Name }
`;

// The page a row opens: it declares `id`, so the row's id flows in.
const detail = `title: Wallaby detail
params:
- name: id
loaders:
  rows:
    sql: SELECT 'Habitat of ' || :id AS habitat UNION ALL SELECT 'Diet of ' || :id
view:
  id: rows
  viz: table
  from: rows
  columns: [{ key: habitat, label: Habitat }]
`;

test("a row expands to the expand page's rows, given that row's values, and folds back", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-expand", { files: { "pages/wallaby.yaml": list, "parts/wallaby-detail.yaml": detail } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_expand/wallaby");
  const host = shown(page).getByTestId("page-host");
  await expect(host).toContainText("Bilby");
  await expect(host).not.toContainText("Habitat of");

  // Rows by id, descending: wallaby, then bilby.
  await host.getByLabel("Expand").nth(1).click();
  await expect(host).toContainText("Habitat of bilby");
  await expect(host).toContainText("Diet of bilby");
  await expect(host).not.toContainText("of wallaby");

  await host.getByLabel("Expand").first().click();
  await expect(host).toContainText("Habitat of wallaby");

  // Collapsing one row leaves the other open.
  await host.getByLabel("Collapse").first().click();
  await expect(host).not.toContainText("of wallaby");
  await expect(host).toContainText("Habitat of bilby");
});
