import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// A replacement takes the target's whole props contract: `vcs.history.graph`,
// Git History's list of commits, is the foundation's replaceable node.
const graph = `title: Wombat history
params:
- name: head_sha
- name: reveal_sha
- name: limit
loaders:
  mine:
    sql: SELECT 'Wombat history of ' || coalesce(:limit, 'nothing') AS line
view:
  id: mine
  viz: table
  from: mine
  columns: [{ key: line, label: Line }]
`;

test("a replaceable node shows the active replacement, and oxplow's own once it is off", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "wombat.txt"), "wombat\n");
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  git("add", "wombat.txt");
  git("commit", "-q", "-m", "Wombat own history row");
  await open(page, daemon, "oxplow:oxplow/page?page=git-history");
  const host = shown(page).getByTestId("page-host");
  // Nothing replaces the list yet: oxplow's own shows.
  await expect(host.getByTestId("node-history")).toContainText("Wombat own history row");
  await expect(shown(page).getByTestId("replacement-vcs.history.graph")).toHaveCount(0);

  await installExtension(daemon, "e2e-replacement", {
    manifest: "ui:\n  mounts:\n    - { page: wombat-history, at: vcs.history.graph, version: 2 }\n",
    files: { "parts/wombat-history.yaml": graph },
  });
  const replaced = host.getByTestId("replacement-vcs.history.graph");
  await expect(replaced).toContainText("Wombat history of");
  await expect(replaced).toHaveAttribute("title", /by e2e-replacement/);
  await expect(host.getByTestId("node-history")).not.toContainText("Wombat own history row");

  // Off, the page is oxplow's own again.
  await run(daemon, "oxplow.extension.disable", { name: "e2e-replacement" });
  await expect(host.getByTestId("node-history")).toContainText("Wombat own history row");
  await expect(replaced).toHaveCount(0);
});
