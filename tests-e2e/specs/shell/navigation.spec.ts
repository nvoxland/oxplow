import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher, workItemIdOf } from "../../support/ui.js";

/// A wiki page with a link to a task: the page, then the task in the same tab.
async function wikiWithTask(daemon: Parameters<typeof run>[0] & { threadUri: string }, slug: string, task: string) {
  const created = await run<{ result: { ref: string } }>(daemon, "oxplow.work_item.create", {
    title: task,
    thread: daemon.threadUri,
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug,
    title: `${slug} page`,
    body: `# ${slug}\n\nFirst do [[${workItemIdOf(created.result.ref)}]].\n`,
  });
  await searchable(daemon, slug, "wiki");
  return created.result.ref;
}

async function openWiki(page: import("@playwright/test").Page, slug: string) {
  await searchLauncher(page, `${slug} page`);
  await page.getByTestId(`launcher-hit-oxplow:oxplow/wiki?slug=${slug}`).click();
  await expect(page.getByTestId("page-wiki")).toContainText("First do");
}

test("Cmd+[ goes back through the tab's pages and Cmd+] goes forward", async ({ page, daemon }) => {
  const ref = await wikiWithTask(daemon, "kestrel", "Kestrel follow-up");
  await page.goto("/");
  await openWiki(page, "kestrel");
  await page.getByTestId("page-wiki").locator(`a[href="${ref}"]`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();

  await page.keyboard.press("Meta+[");
  await expect(page.getByTestId("page-wiki").filter({ visible: true })).toContainText("First do");
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeHidden();

  await page.keyboard.press("Meta+]");
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
});

test("the active tab and its history survive a reload", async ({ page, daemon }) => {
  const ref = await wikiWithTask(daemon, "osprey", "Osprey follow-up");
  await page.goto("/");
  await openWiki(page, "osprey");
  await page.getByTestId("page-wiki").locator(`a[href="${ref}"]`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();

  await page.reload();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
  await page.keyboard.press("Meta+[");
  await expect(page.getByTestId("page-wiki").filter({ visible: true })).toContainText("First do");
});

test("Alt+Down steps to the next file of the list it was opened from", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "aa.txt"), "first\n");
  writeFileSync(join(daemon.project, "bb.txt"), "second\n");
  await page.goto("/");
  await openFromLauncher(page, "Files");
  await page.getByTestId("file-tree-entry-aa.txt").click();
  // The file has opened once its editor is up; focus then rests on the tab,
  // not in the editor (which keeps the keys for typing).
  await expect(page.locator(".monaco-editor").first()).toBeVisible();
  await page.getByTestId("center-tab-oxplow:oxplow/file?path=aa.txt").click();

  await page.keyboard.press("Alt+ArrowDown");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=bb.txt")).toBeVisible();
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=aa.txt")).toBeHidden();
});
