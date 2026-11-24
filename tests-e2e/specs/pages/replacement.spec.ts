import { fileTask, run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// A replacement declares the target's whole props contract: `task.list`, the
// Task List's list, is oxplow-bundled's replaceable node and takes `scope`
// and `thread_id` (the Task List gives neither; the contract itself is the
// Rust tests', `replacement.missing_params`).
const list = `title: Wombat list
params:
- name: scope
- name: thread_id
loaders:
  mine:
    sql: SELECT 'Wombat list' || coalesce(' of ' || :scope, '') || coalesce(' for ' || :thread_id, '') AS line
view:
  id: mine
  viz: table
  from: mine
  columns: [{ key: line, label: Line }]
`;

test("a replaceable node shows the active replacement, and oxplow's own once it is off", async ({ page, daemon }) => {
  const chooseList = (extension: string) => run(daemon, "oxplow.config.set", {
    key: "oxplow.replacements",
    level: "project",
    value: { "task.list": extension },
  });
  await chooseList("oxplow");
  await fileTask(daemon, "Wombat own list row");
  await open(page, daemon, "oxplow:oxplow/page?page=task-list");
  const host = shown(page).getByTestId("page-host");
  // The project's selection keeps the built-in list.
  await expect(host.getByTestId("node-list")).toContainText("Wombat own list row");
  await expect(shown(page).getByTestId("replacement-task.list")).toHaveCount(0);

  await installExtension(daemon, "e2e-replacement", {
    manifest: "ui:\n  mounts:\n    - { page: wombat-list, at: task.list, version: 2 }\n",
    files: { "parts/wombat-list.yaml": list },
  });
  await chooseList("e2e-replacement");
  const replaced = host.getByTestId("replacement-task.list");
  await expect(replaced).toContainText("Wombat list");
  await expect(replaced).toHaveAttribute("title", /by e2e-replacement/);
  await expect(host.getByTestId("node-list")).not.toContainText("Wombat own list row");

  // Off, the page is oxplow's own again.
  await run(daemon, "oxplow.extension.disable", { name: "e2e-replacement" });
  await expect(host.getByTestId("node-list")).toContainText("Wombat own list row");
  await expect(replaced).toHaveCount(0);
});
