import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown, reload } from "../../support/ui.js";

const spec = `title: Select Marsupial
category: Data
params:
- name: animal
  label: Animal
loaders:
  animals:
    sql: SELECT 'quokka' AS id, 'Quokka' AS name UNION ALL SELECT 'numbat', 'Numbat' ORDER BY 1 DESC
  facts:
    sql: SELECT 'Fact about ' || :animal AS fact WHERE :animal IS NOT NULL
view:
  stack:
  - id: animals
    title: Animals
    viz: table
    from: animals
    columns:
    - { key: id, label: Id, select: animal }
    - { key: name, label: Name }
  - id: facts
    title: Facts
    viz: table
    from: facts
    columns: [{ key: fact, label: Fact }]
`;

test("a row click writes the page's param and a section that reads it fills", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-select", { files: { "pages/marsupial.yaml": spec } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_select/marsupial");
  const host = shown(page).getByTestId("page-host");
  const animals = host.locator('[data-node="animals"]');
  const facts = host.locator('[data-node="facts"]');
  await expect(animals).toContainText("Numbat");
  // Nothing is picked: the section stays, with nothing in it yet.
  await expect(facts.getByTestId("viz-empty")).toContainText("No rows");
  // A selection is never an input in the rail.
  await expect(host.getByTestId("page-rail").getByTestId("page-param-animal")).toHaveCount(0);

  const row = animals.getByTestId("viz-row-1");
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(facts).toContainText("Fact about numbat");
  // The pick is in the tab's address, so a reload keeps it.
  await expect(shown(page)).toHaveAttribute("data-testid", /animal=numbat/);
  await reload(page);
  await expect(facts).toContainText("Fact about numbat");

  // Another row replaces the pick.
  await animals.getByTestId("viz-row-0").click();
  await expect(facts).toContainText("Fact about quokka");
  await expect(facts).not.toContainText("Fact about numbat");
});
