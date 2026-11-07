import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { expandRailSection, load } from "../../support/ui.js";

const body = `title: Glider body
loaders:
  gliders:
    sql: SELECT title FROM v_work_item WHERE title LIKE 'Glider %' ORDER BY title
view:
  id: gliders
  viz: table
  from: gliders
  columns: [{ key: title, label: Glider }]
  empty: No gliders yet.
`;

const count = `title: Glider count
loaders:
  n:
    sql: SELECT count(*) AS n FROM v_work_item WHERE title LIKE 'Glider %'
badge: { loader: n, column: n }
view: { id: n, viz: number, from: n }
`;

test("a left-nav panel draws its body page, and its badge page's number", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-panel", {
    manifest: "panels:\n  - { id: gliders, title: Gliders, icon: bell, scope: project, body: glider-body, badge: glider-count }\n",
    files: { "parts/glider-body.yaml": body, "parts/glider-count.yaml": count },
  });
  await load(page);
  const id = "ext:e2e-panel/gliders";
  await expandRailSection(page, id);
  const section = page.getByTestId(`rail-section-${id}`);
  await expect(section).toContainText("Gliders");
  await expect(section.getByTestId("rail-panel-body")).toContainText("No gliders yet.");
  await expect(section.getByTestId(`rail-section-toggle-${id}`)).not.toContainText("1");

  // A write the body reads shows in it, and the badge counts it.
  await run(daemon, "oxplow.work_item.create", { title: "Glider sugar", thread: daemon.thread });
  await expect(section.getByTestId("rail-panel-body")).toContainText("Glider sugar");
  await expect(section.getByTestId(`rail-section-toggle-${id}`)).toContainText("1");
});
