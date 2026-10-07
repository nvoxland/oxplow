import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("search finds a task and a wiki page, and opens them", async ({ page, daemon }) => {
  const created = await run<{ result: { id: string } }>(daemon, "oxplow.work_item.create", {
    title: "Quokka migration",
    thread: daemon.thread,
  });
  await run(daemon, "oxplow.knowledge.write_page", { slug: "quokka-notes", title: "Quokka notes", body: "# Quokka notes\n\nWhat we know.\n" });
  await searchable(daemon, "quokka", "task");
  await searchable(daemon, "quokka", "wiki");
  await page.goto("/");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("quokka");
  await page.getByTestId(`launcher-hit-task:${created.result.id}`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
  await expect(page.locator("body")).toContainText("Quokka migration");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("quokka");
  await page.getByTestId("launcher-hit-wiki:quokka-notes").click();
  await expect(page.getByTestId("page-wiki")).toContainText("What we know.");
});

// tsk1030: an extension added while the page is open — the agent writes
// one — shows in the launcher without a reload.
test("a lens added while the page is open is in the launcher", async ({ page, daemon }) => {
  await page.goto("/");
  // The launcher has loaded the extensions once already.
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("Wombat");
  await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  const dir = join(daemon.project, "oxplow", "extensions", "wombat");
  mkdirSync(join(dir, "lenses"), { recursive: true });
  writeFileSync(
    join(dir, "extension.yaml"),
    "manifest: 2\nname: wombat\nsharing: private\nintent:\n  purpose: Wombat tasks.\n  origin: null\n  examples: []\n",
  );
  writeFileSync(
    join(dir, "lenses", "wombats.yaml"),
    "title: Wombat Tasks\nquery: SELECT id FROM v_task\nviz: table\nlauncher: { category: Work }\n",
  );
  await expect(async () => {
    await page.getByTestId("title-bar-search").click();
    await page.keyboard.type("Wombat Tasks");
    await expect(page.locator('[data-testid^="launcher-page-"]').filter({ hasText: "Wombat Tasks" })).toHaveCount(1, {
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});
