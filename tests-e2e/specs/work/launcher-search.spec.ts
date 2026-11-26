import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { deploy, extensionSource, fileTask, run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher, load, tabOf } from "../../support/ui.js";

test("search finds a task and a knowledge page, and opens them", async ({ page, daemon }) => {
  const created = await fileTask(daemon, "Quokka migration");
  await run(daemon, "oxwiki.page.write", { id: "quokka-notes", title: "Quokka notes", body: "# Quokka notes\n\nWhat we know.\n" });
  await searchable(daemon, "quokka", "task");
  await searchable(daemon, "quokka", "knowledge");
  await load(page);
  // A hit opens the foundation's resource page for its URI.
  await searchLauncher(page, "quokka");
  await page.getByTestId(`launcher-hit-${created.result.uri}`).click();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toContainText("Quokka migration");
  await searchLauncher(page, "quokka");
  await page.getByTestId("launcher-hit-oxplow:oxplow/knowledge?id=quokka-notes&store=oxwiki").click();
  await expect(tabOf(page, "oxplow:oxplow/knowledge?id=quokka-notes&store=oxwiki")).toBeVisible();
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toContainText("Quokka notes");
});

// An extension added while the page is open — the agent writes
// one — shows in the launcher without a reload.
test("a page added while the window is open is in the launcher", async ({ page, daemon }) => {
  await load(page);
  // The launcher has loaded the extensions once already.
  await searchLauncher(page, "Wombat");
  await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat" })).not.toBeVisible();
  await page.keyboard.press("Escape");
  const dir = extensionSource(daemon, "wombat");
  mkdirSync(join(dir, "pages"), { recursive: true });
  writeFileSync(
    join(dir, "extension.yaml"),
    "manifest: 2\nname: wombat\nintent:\n  purpose: Wombat tasks.\n  origin: null\n  examples: []\n",
  );
  writeFileSync(
    join(dir, "pages", "wombats.yaml"),
    "title: Wombat Tasks\ncategory: Work\nloaders:\n  items: { sql: SELECT uri FROM v_oxplow_task }\nview: { from: items, viz: table }\n",
  );
  await deploy(daemon, "wombat");
  await expect(async () => {
    await searchLauncher(page, "Wombat Tasks");
    await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat Tasks" })).toHaveCount(1, {
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});
