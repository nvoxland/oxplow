import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A `role: view` param is state the page or the system sets: it rides the
// `#` like a filter but the rail never draws it. A page with no other param
// has no Filters group (params.spec checks that a filter is drawn).

const numbat = `title: Viewed Numbat
category: Data
params:
- name: pane
  role: view
  default: first
loaders:
  rows:
    sql: SELECT 'Numbat in ' || :pane AS line
view:
  viz: table
  from: rows
`;

test("a page whose only param is a view param has no Filters group and draws no input for it", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-viewrole", { files: { "pages/numbat.yaml": numbat } });
  await page.goto("/");
  await openFromLauncher(page, "Viewed Numbat");
  const file = page.getByTestId("page-file").filter({ visible: true });
  await expect(file).toContainText("Numbat in first");
  await expect(page.getByTestId("page-rail-filters").filter({ visible: true })).toHaveCount(0);
  await expect(page.getByTestId("page-param-pane").filter({ visible: true })).toHaveCount(0);
});
