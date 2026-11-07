import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

const spec = `title: Form Numbat
category: Data
loaders:
  filed:
    sql: SELECT title FROM v_work_item WHERE title LIKE 'Form numbat %' ORDER BY title
view:
  id: filed
  viz: table
  from: filed
  columns: [{ key: title, label: Filed }]
  empty: Nothing filed yet.
actions:
- id: file
  label: File a numbat
  command: oxplow.work_item.create
  prominent: true
  form: true
  input:
    title: 'Form numbat '
`;

test("a page's form action submits, and the page shows the result", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-form", {
    manifest: "permissions: [commands:oxplow.work_item.create]\n",
    files: { "pages/numbat.yaml": spec },
  });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-form/numbat");
  const host = shown(page).getByTestId("page-host");
  await expect(host).toContainText("Nothing filed yet.");
  await shown(page).getByTestId("action-page-action-file").click();
  const title = shown(page).getByTestId("viz-action-form-fields-title");
  await expect(title).toHaveValue("Form numbat ");
  await title.fill("Form numbat one");
  await title.press("Enter");
  // The form closes and the loader re-runs on the new item.
  await expect(title).toHaveCount(0);
  await expect(host).toContainText("Form numbat one");
  await expect(host).not.toContainText("Nothing filed yet.");
});
