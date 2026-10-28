import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const outer = `title: Embed Echidna
category: Data
loaders:
  intro:
    sql: SELECT 'Outer echidna' AS line
view:
  stack:
  - { id: intro, title: Intro, viz: table, from: intro, columns: [{ key: line, label: Line }] }
  - { id: inner, embed: { page: e2e-embed/echidna-inner, params: { name: spiny } } }
`;

const inner = `title: Echidna inner
hidden: true
params:
- name: name
loaders:
  facts:
    sql: SELECT 'Inner ' || :name AS line
view:
  stack:
  - { id: facts, title: Facts, viz: table, from: facts, columns: [{ key: line, label: Line }] }
  - { id: note, title: Note, markdown: The embedded note. }
`;

test("an embedded page's nodes show in the host, with ids <embed>/<child>", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-embed", { files: { "pages/echidna.yaml": outer, "pages/echidna-inner.yaml": inner } });
  await page.goto("/");
  await openFromLauncher(page, "Embed Echidna");
  const host = page.getByTestId("page-host").filter({ visible: true });
  await expect(host.getByTestId("node-intro")).toContainText("Outer echidna");
  // The embedded page's nodes keep the embed's id as a prefix, and read the
  // params the embed gave.
  await expect(host.getByTestId("node-inner/facts")).toContainText("Inner spiny");
  await expect(host.getByTestId("node-inner/note")).toContainText("The embedded note.");
  await expect(host.locator('[data-node="inner/facts"]')).toHaveCount(1);
});
