import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { deploy, extensionSource, run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

test("search finds a task and a wiki page, and opens them", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Quokka migration",
    thread: daemon.thread,
  });
  await run(daemon, "oxplow.knowledge.write_page", { slug: "quokka-notes", title: "Quokka notes", body: "# Quokka notes\n\nWhat we know.\n" });
  await searchable(daemon, "quokka", "work_item");
  await searchable(daemon, "quokka", "wiki");
  await page.goto("/");
  await searchLauncher(page, "quokka");
  await page.getByTestId(`launcher-hit-${created.result.uri}`).click();
  await expect(page.locator('[data-node="state"]')).toBeVisible();
  await expect(page.locator("body")).toContainText("Quokka migration");
  await searchLauncher(page, "quokka");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=quokka-notes").click();
  await expect(page.locator('[data-testid="page-file"]:visible')).toContainText("What we know.");
});

// tsk1030: an extension added while the page is open — the agent writes
// one — shows in the launcher without a reload.
test("a lens added while the page is open is in the launcher", async ({ page, daemon }) => {
  await page.goto("/");
  // The launcher has loaded the extensions once already.
  await searchLauncher(page, "Wombat");
  await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  const dir = extensionSource(daemon, "wombat");
  mkdirSync(join(dir, "lenses"), { recursive: true });
  writeFileSync(
    join(dir, "extension.yaml"),
    "manifest: 2\nname: wombat\nintent:\n  purpose: Wombat tasks.\n  origin: null\n  examples: []\n",
  );
  writeFileSync(
    join(dir, "lenses", "wombats.yaml"),
    "title: Wombat Tasks\nquery: SELECT uri FROM v_work_item\nviz: table\nlauncher: { category: Work }\n",
  );
  await deploy(daemon, "wombat");
  await expect(async () => {
    await searchLauncher(page, "Wombat Tasks");
    await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat Tasks" })).toHaveCount(1, {
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});
