import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A replacement takes the target's whole props contract.
const lanes = `title: Wombat task list
params:
- name: scope
- name: thread_id
loaders:
  mine:
    sql: SELECT 'Wombat list for ' || coalesce(:scope, 'nothing') AS line
view:
  id: mine
  viz: table
  from: mine
  columns: [{ key: line, label: Line }]
`;

test("a replaceable node shows the active replacement, and oxplow's own once it is off", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Wombat own list row", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  const host = page.getByTestId("page-host").filter({ visible: true });
  // Nothing replaces the list yet: oxplow's own shows.
  await expect(host.getByTestId("node-list")).toContainText("Wombat own list row");
  await expect(page.getByTestId("replacement-work_item.list")).toHaveCount(0);

  await installExtension(daemon, "e2e-replacement", {
    manifest: "ui:\n  mounts:\n    - { page: wombat-list, at: work_item.list, version: 1 }\n",
    files: { "parts/wombat-list.yaml": lanes },
  });
  const replaced = host.getByTestId("replacement-work_item.list");
  await expect(replaced).toContainText("Wombat list for thread");
  await expect(replaced.getByTestId("replacement-badge")).toContainText("replaced by e2e-replacement");
  await expect(host.getByTestId("node-list")).not.toContainText("Wombat own list row");

  // Off, the page is oxplow's own again.
  await run(daemon, "oxplow.extension.disable", { name: "e2e-replacement" });
  await expect(host.getByTestId("node-list")).toContainText("Wombat own list row");
  await expect(replaced).toHaveCount(0);
});
