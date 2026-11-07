import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// `viz: sections`: a subsection per row — a heading one level below its
// parent (an arrow after it opens the row's page when it has one), facts, a
// markdown body — listed in Contents under the parent, the first ten; more
// rows end in one "… N more" that scrolls to the parent.

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

test("a subsection per row, headed below its parent, with its arrow, facts and body, and in Contents", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-sections", { files: { "pages/marsupial.yaml": few } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-sections/marsupial");
  const host = shown(page).getByTestId("page-host");
  await expect(host.locator("h2.ox-section-title")).toHaveText(["Animals"]);
  await expect(host.locator("h3.ox-section-title")).toHaveText(["Quokka", "Kangaroo", "Wombat", "Koala"]);
  // The heading text is plain; Quokka has a page, so an arrow follows it.
  await expect(host.locator('[data-node="animals/quokka"] h3')).toHaveText("Quokka");
  const arrow = host.getByRole("button", { name: "Open Quokka" });
  await expect(arrow).toHaveAttribute("title", "Open Quokka");
  await expect(host.locator('[data-node="animals/quokka"] h3').getByTestId("open-page-link")).toHaveCount(1);
  await expect(host.locator('[data-node="animals/kangaroo"] h3').getByTestId("open-page-link")).toHaveCount(0);
  await expect(host.getByTestId("viz-section-facts-quokka")).toContainText("Small");
  await expect(host.getByTestId("viz-section-body-quokka").locator("strong")).toHaveText("a lot");

  // Five sections in all: Contents lists the parent and its subsections.
  const contents = shown(page).getByTestId("viz-rail-contents");
  await expect(contents.locator("button")).toHaveText(["Animals", "Quokka", "Kangaroo", "Wombat", "Koala"]);
});

test("more subsections than the cap are drawn, but Contents lists ten, then how many more", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-sections-long", { files: { "pages/long.yaml": many } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-sections-long/long");
  const host = shown(page).getByTestId("page-host");
  await expect(host.locator("h3.ox-section-title")).toHaveCount(14);
  const contents = shown(page).getByTestId("viz-rail-contents");
  await expect(contents.locator("button")).toHaveText([
    "First",
    "Second",
    "Third",
    "Pages",
    ...Array.from({ length: 10 }, (_, i) => `Page ${i + 1}`),
    "… 4 more",
  ]);
});

const grouped = `title: Sections Grouped
category: Data
loaders:
  rows:
    sql: |
      SELECT 'p1' AS id, 'P1' AS name, 'Alpha' AS owner, 'oxplow:oxplow/page?page=git-dashboard' AS owner_uri
      UNION ALL SELECT 'q1', 'Q1', 'Beta', NULL
      UNION ALL SELECT 'p2', 'P2', 'Alpha', 'oxplow:oxplow/page?page=git-dashboard'
view:
  stack:
    - id: probs
      title: Problems
      viz: sections
      from: rows
      sections: { title: name, id: id, group: owner, group_link: owner_uri }
`;

test("grouped rows nest under their group's heading, with the arrow on the group, and Contents nests both levels", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-sections-grouped", { files: { "pages/grouped.yaml": grouped } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-sections-grouped/grouped");
  const host = shown(page).getByTestId("page-host");
  await expect(host.locator("h2.ox-section-title")).toHaveText(["Problems"]);
  await expect(host.locator("h3.ox-section-title")).toHaveText(["Alpha", "Beta"]);
  await expect(host.locator("h4.ox-section-title")).toHaveText(["P1", "P2", "Q1"]);
  await expect(host.getByRole("button", { name: "Open Alpha" })).toHaveCount(1);
  await expect(host.getByRole("button", { name: "Open Beta" })).toHaveCount(0);
  const contents = shown(page).getByTestId("viz-rail-contents");
  await expect(contents.locator("button")).toHaveText(["Problems", "Alpha", "P1", "P2", "Beta", "Q1"]);
});
