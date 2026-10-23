import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher, workItemIdOf } from "../../support/ui.js";

/// A wiki page with a link to a task: the page, then the task in the same tab.
async function wikiWithTask(daemon: Parameters<typeof run>[0] & { thread: string }, slug: string, task: string) {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: task,
    thread: daemon.thread,
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug,
    title: `${slug} page`,
    body: `# ${slug}\n\nFirst do [[${workItemIdOf(created.result.uri)}]].\n`,
  });
  await searchable(daemon, slug, "wiki");
  return created.result.uri;
}

async function openWiki(page: import("@playwright/test").Page, slug: string) {
  await searchLauncher(page, `${slug} page`);
  await page.getByTestId(`launcher-hit-oxplow:oxplow/wiki?slug=${slug}`).click();
  await expect(page.locator('[data-testid="page-lens"]:visible')).toContainText("First do");
}

test("Cmd+[ goes back through the tab's pages and Cmd+] goes forward", async ({ page, daemon }) => {
  const ref = await wikiWithTask(daemon, "kestrel", "Kestrel follow-up");
  await page.goto("/");
  await openWiki(page, "kestrel");
  await page.locator('[data-testid="page-lens"]:visible').locator(`a[href="${ref}"]`).click();
  await expect(page.getByTestId("lens-part-state")).toBeVisible();

  await page.keyboard.press("Meta+[");
  await expect(page.locator('[data-testid="page-lens"]:visible')).toContainText("First do");
  await expect(page.getByTestId("lens-part-state")).toBeHidden();

  await page.keyboard.press("Meta+]");
  await expect(page.getByTestId("lens-part-state")).toBeVisible();
});

test("the active tab and its history survive a reload", async ({ page, daemon }) => {
  const ref = await wikiWithTask(daemon, "osprey", "Osprey follow-up");
  await page.goto("/");
  await openWiki(page, "osprey");
  await page.locator('[data-testid="page-lens"]:visible').locator(`a[href="${ref}"]`).click();
  await expect(page.getByTestId("lens-part-state")).toBeVisible();

  await page.reload();
  await expect(page.getByTestId("lens-part-state")).toBeVisible();
  await page.keyboard.press("Meta+[");
  await expect(page.locator('[data-testid="page-lens"]:visible')).toContainText("First do");
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

const navigate = (daemon: Parameters<typeof run>[0], uri: string, extra: Record<string, unknown> = {}) =>
  run(daemon, "oxplow.window.navigate", { uri, ...extra });
const backButton = (page: import("@playwright/test").Page) => page.getByTestId("page-nav-back").filter({ visible: true });
const shownBody = (page: import("@playwright/test").Page, needle: string) =>
  page.locator(`[data-testid^="center-tab-body-oxplow:oxplow/"][data-testid*="${needle}"]:visible`);

// The same page in another view is the same tab: navigating to it with a
// new filter focuses the tab that has it, and never opens a second.
test("opening a page again with another filter focuses its tab", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  const tabs = page.locator('[data-testid^="center-tab-oxplow:oxplow/page?page=tasks"]');
  await expect(tabs).toHaveCount(1);
  await navigate(daemon, "oxplow:oxplow/page?page=tasks#scope=all", { new_tab: true });
  await expect(shownBody(page, "page=tasks#scope=all")).toHaveCount(1);
  await expect(tabs).toHaveCount(1);
  // The launcher finds it too.
  await openFromLauncher(page, "Tasks");
  await expect(tabs).toHaveCount(1);
});

// A link is followed in the shown tab (Back returns), unless `replace`
// takes the shown page's place.
test("window.navigate pushes by default and replaces on request", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  await expect(shownBody(page, "page=tasks")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();

  await navigate(daemon, "oxplow:oxplow/page?page=settings");
  await expect(shownBody(page, "page=settings")).toHaveCount(1);
  await expect(backButton(page)).toBeEnabled();

  await navigate(daemon, "oxplow:oxplow/page?page=local-history", { replace: true });
  await expect(shownBody(page, "page=local-history")).toHaveCount(1);
  // Settings was replaced: Back skips it, to Tasks.
  await backButton(page).click();
  await expect(shownBody(page, "page=tasks")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();
});

// A file at a line is the file's view (`#lines=`): another line is another
// entry of the same tab, and Back returns to the first.
test("a file opened at a line lands at its #lines view, and Back returns to the line before", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "lines.txt"), Array.from({ length: 12 }, (_, i) => `line ${i + 1}`).join("\n") + "\n");
  await page.goto("/");
  // The window answers a navigate once its stream is shown: a page open is the sign.
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  await navigate(daemon, "oxplow:oxplow/file?path=lines.txt#lines=3-3");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=3-3")).toBeVisible();
  await expect(page.locator(".monaco-editor").first()).toBeVisible();

  await navigate(daemon, "oxplow:oxplow/file?path=lines.txt#lines=8-8");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=8-8")).toBeVisible();
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/file?path=lines.txt"]')).toHaveCount(1);

  await backButton(page).click();
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=3-3")).toBeVisible();
});
