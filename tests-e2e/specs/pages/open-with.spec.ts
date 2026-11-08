import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open } from "../../support/ui.js";

// The ☰ menu of a page: Open with is offered only where a URI has more than
// one renderer, and a page action is offered only while its `when` holds.
const manifest = `datasets:
  - name: thing
    version: 1
    key: [uri]
    description: "One thing, as an \`e2e_one/thing\` URI."
    fields:
      - { name: uri, type: TEXT, doc: "The thing's URI." }
      - { name: title, type: TEXT, doc: "Its title." }
resources:
  - name: thing
    label: Thing
    params: { id: { required: true, pattern: '^[0-9]+$' } }
    resolve: thing
    icon: box
`;
const dataset = `SELECT CAST(uri('e2e_one/thing', 'id', '1') AS TEXT) AS uri, CAST('Only thing' AS TEXT) AS title
`;
const thing = `title: Thing
description: The one renderer of a thing.
for: e2e_one/thing
default: true
params:
- name: uri
loaders:
  thing:
    sql: SELECT title FROM v_e2e_one_thing WHERE uri = :uri
view: { from: thing, viz: table }
`;
const gated = `title: Gated Numbat
category: Data
loaders:
  state:
    sql: SELECT 1 AS shown, 0 AS hidden
view: { from: state, viz: table }
actions:
- id: when-true
  label: Shown action
  command: oxplow.window.navigate
  input: { uri: 'oxplow:e2e_one/thing?id=1' }
  when: shown
- id: when-false
  label: Hidden action
  command: oxplow.window.navigate
  input: { uri: 'oxplow:e2e_one/thing?id=1' }
  when: hidden
`;

test("Open with is absent for a URI with one renderer, and an action whose `when` is false is not in the menu", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-one", {
    manifest: manifest.replace("datasets:", "permissions: [commands:oxplow.window.navigate]\ndatasets:"),
    files: { "datasets/thing.sql": dataset, "pages/thing.yaml": thing, "pages/gated.yaml": gated },
  });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_one/gated");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-page-action-when-true")).toBeVisible();
  await expect(page.getByTestId("menu-item-action-page-action-when-false")).not.toBeVisible();
  await page.keyboard.press("Escape");

  await run(daemon, "oxplow.window.navigate", { uri: "oxplow:e2e_one/thing?id=1" });
  await expect(page.getByTestId("center-tab-oxplow:e2e_one/thing?id=1")).toBeVisible();
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-page-about")).toBeVisible();
  await expect(page.getByTestId("menu-item-page-open-with")).not.toBeVisible();
});
