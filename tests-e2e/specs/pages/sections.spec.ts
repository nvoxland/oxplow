import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// `viz: sections`: a subsection per row — a linked heading one level below
// its parent, facts, a markdown body — listed in Contents under the
// parent, up to the cap (12); more rows, and only the parent is listed.

const few = `title: Sections Marsupial
category: Data
loaders:
  animals:
    sql: |
      SELECT 'quokka' AS id, 'Quokka' AS name, 'oxplow:oxplow/page?page=git-dashboard' AS uri, 'Small' AS size, 'Smiles **a lot**.' AS note
      UNION ALL SELECT 'kangaroo', 'Kangaroo', NULL, 'Large', NULL
      UNION ALL SELECT 'wombat', 'Wombat', NULL, 'Medium', 'Digs.'
      UNION ALL SELECT 'koala', 'Koala', NULL, 'Medium', NULL
view:
  stack:
    - id: animals
      title: Animals
      viz: sections
      from: animals
      sections: { title: name, id: id, link: uri, body: note }
      columns: [{ key: size, label: Size }]
`;

const many = `title: Sections Long
category: Data
loaders:
  n:
    sql: |
      WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM c WHERE i < 14)
      SELECT 'p' || i AS id, 'Page ' || i AS name, 'About page ' || i AS note FROM c
  one:
    sql: SELECT 1 AS a
view:
  stack:
    - { id: first, title: First, viz: markdown, from: one, markdown: { text: a } }
    - { id: second, title: Second, viz: markdown, from: one, markdown: { text: a } }
    - { id: third, title: Third, viz: markdown, from: one, markdown: { text: a } }
    - id: pages
      title: Pages
      viz: sections
      from: n
      sections: { title: name, id: id, body: note }
`;

test("a subsection per row, headed below its parent, with its link, facts and body, and in Contents", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-sections", { files: { "pages/marsupial.yaml": few } });
  await page.goto("/");
  await openFromLauncher(page, "Sections Marsupial");
  const host = page.getByTestId("page-host").filter({ visible: true });
  await expect(host.locator("h2.ox-section-title")).toHaveText(["Animals"]);
  await expect(host.locator("h3.ox-section-title")).toHaveText(["Quokka", "Kangaroo", "Wombat", "Koala"]);
  await expect(host.locator('[data-node="animals/quokka"] h3 a, [data-node="animals/quokka"] h3 button')).toHaveCount(1);
  await expect(host.locator('[data-node="animals/kangaroo"] h3 a, [data-node="animals/kangaroo"] h3 button')).toHaveCount(0);
  await expect(host.getByTestId("viz-section-facts-quokka")).toContainText("Small");
  await expect(host.getByTestId("viz-section-body-quokka").locator("strong")).toHaveText("a lot");

  // Five sections in all: Contents lists the parent and its subsections.
  const contents = page.getByTestId("viz-rail-contents").filter({ visible: true });
  await expect(contents.locator("button")).toHaveText(["Animals", "Quokka", "Kangaroo", "Wombat", "Koala"]);
});

test("more subsections than the cap are drawn, but Contents lists only their parent", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-sections-long", { files: { "pages/long.yaml": many } });
  await page.goto("/");
  await openFromLauncher(page, "Sections Long");
  const host = page.getByTestId("page-host").filter({ visible: true });
  await expect(host.locator("h3.ox-section-title")).toHaveCount(14);
  const contents = page.getByTestId("viz-rail-contents").filter({ visible: true });
  await expect(contents.locator("button")).toHaveText(["First", "Second", "Third", "Pages"]);
});
