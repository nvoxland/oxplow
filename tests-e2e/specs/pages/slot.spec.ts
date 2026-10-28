import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const host = `title: Slot Possum
category: Data
loaders:
  possum:
    sql: SELECT 'ringtail' AS kind
view:
  stack:
  - { id: first, markdown: The host's own section. }
  - { slot: extras }
`;

// The mounted page reads \`kind\`, a column of the host's first loader row.
const mounted = `title: Possum extras
hidden: true
params:
- name: kind
loaders:
  extra:
    sql: SELECT 'Mounted ' || :kind AS line
view:
  id: extra
  viz: table
  from: extra
  columns: [{ key: line, label: Line }]
`;

test("a page mounted at another page's slot shows inside it, given the host's values", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-slot-host", { files: { "pages/possum.yaml": host } });
  await installExtension(daemon, "e2e-slot-guest", {
    manifest: "ui:\n  mounts:\n    - { page: possum-extras, at: e2e-slot-host/possum.extras, title: Extras }\n",
    files: { "pages/possum-extras.yaml": mounted },
  });
  await page.goto("/");
  await openFromLauncher(page, "Slot Possum");
  const body = page.getByTestId("page-host").filter({ visible: true });
  await expect(body.getByTestId("node-first")).toContainText("The host's own section.");
  // The mount is drawn in the host, its nodes under the mounted page's id
  // (`/` written `.`), and it read the host's row.
  const mount = body.locator('[data-node^="e2e-slot-guest.possum-extras"]').first();
  await expect(mount).toContainText("Mounted ringtail");

  // With the mounting extension off the slot draws nothing.
  await run(daemon, "oxplow.extension.disable", { name: "e2e-slot-guest" });
  await expect(mount).toHaveCount(0);
  await expect(body.getByTestId("node-first")).toBeVisible();
});
