import { fileTask } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openPanel, load, panel, panelBody, panelHeader } from "../../support/ui.js";

const body = `title: Glider body
loaders:
  gliders:
    sql: SELECT title FROM v_oxplow_task WHERE title LIKE 'Glider %' ORDER BY title
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
    sql: SELECT count(*) AS n FROM v_oxplow_task WHERE title LIKE 'Glider %'
badge: { loader: n, column: n }
view: { id: n, viz: number, from: n }
`;

test("a left-nav panel draws its body page, and its badge page's number", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-panel", {
    manifest: "requires: [oxplow-bundled]\npanels:\n  - { id: gliders, title: Gliders, icon: bell, data_scope: project, body: glider-body, badge: glider-count }\n",
    files: { "parts/glider-body.yaml": body, "parts/glider-count.yaml": count },
  });
  await load(page);
  const id = "ext:e2e_panel/gliders";
  await openPanel(page, id);
  const section = panel(page, id);
  await expect(section).toContainText("Gliders");
  await expect(panelBody(page, id)).toContainText("No gliders yet.");
  await expect(panelHeader(page, id)).not.toContainText("1");

  // A write the body reads shows in it, and the badge counts it.
  await fileTask(daemon, "Glider sugar");
  await expect(panelBody(page, id)).toContainText("Glider sugar");
  await expect(panelHeader(page, id)).toContainText("1");
});
