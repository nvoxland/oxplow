import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

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
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-viewrole/numbat");
  const file = shown(page).getByTestId("page-file");
  await expect(file).toContainText("Numbat in first");
  await expect(shown(page).getByTestId("page-rail-filters")).toHaveCount(0);
  await expect(shown(page).getByTestId("page-param-pane")).toHaveCount(0);
});
