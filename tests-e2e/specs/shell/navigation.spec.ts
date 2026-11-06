import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const overviewTitle = (page: import("@playwright/test").Page) => page.getByTestId("page-nav-title").filter({ visible: true });

/// An item the test extension's overview lists, each title a link to its page.
async function item(daemon: Parameters<typeof run>[0] & { thread: string }, title: string) {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", { title, thread: daemon.thread });
  return created.result.uri;
}

async function openOverview(page: import("@playwright/test").Page, title: string) {
  await openFromLauncher(page, "Item overview");
  await expect(page.locator('[data-testid="page-file"]:visible')).toContainText(title);
  await expect(overviewTitle(page)).toHaveText("Item overview");
}

async function followItem(page: import("@playwright/test").Page, title: string) {
  await page.locator('[data-testid="page-file"]:visible').getByText(title).click();
  await expect(overviewTitle(page)).toHaveText("Item");
}

test("Cmd+[ goes back through the tab's pages and Cmd+] goes forward", async ({ page, daemon }) => {
  await item(daemon, "Kestrel follow-up");
  await page.goto("/");
  await openOverview(page, "Kestrel follow-up");
  await followItem(page, "Kestrel follow-up");

  await page.keyboard.press("Meta+[");
  await expect(overviewTitle(page)).toHaveText("Item overview");

  await page.keyboard.press("Meta+]");
  await expect(overviewTitle(page)).toHaveText("Item");
});

test("the active tab and its history survive a reload", async ({ page, daemon }) => {
  await item(daemon, "Osprey follow-up");
  await page.goto("/");
  await openOverview(page, "Osprey follow-up");
  await followItem(page, "Osprey follow-up");

  await page.reload();
  await expect(overviewTitle(page)).toHaveText("Item");
  await page.keyboard.press("Meta+[");
  await expect(overviewTitle(page)).toHaveText("Item overview");
});

const fileTree = `title: Tree Heron
category: Data
loaders:
  tree:
    sql: |
      SELECT 'aa.txt' AS id, NULL AS parent, 'aa.txt' AS name, 'oxplow:oxplow/file?path=aa.txt' AS target
      UNION ALL SELECT 'bb.txt', NULL, 'bb.txt', 'oxplow:oxplow/file?path=bb.txt'
view:
  from: tree
  viz: tree
  tree: { id: id, parent: parent, label: name }
  columns:
  - key: name
    label: Name
    link: { kind: page, from: target }
`;

test("Alt+Down steps to the next file of the list it was opened from", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "aa.txt"), "first\n");
  writeFileSync(join(daemon.project, "bb.txt"), "second\n");
  await installExtension(daemon, "e2e-tree", { files: { "pages/heron.yaml": fileTree } });
  await page.goto("/");
  await openFromLauncher(page, "Tree Heron");
  await page.locator('[data-testid="page-file"]:visible').getByRole("button", { name: "aa.txt" }).filter({ visible: true }).click();
  // The file has opened once its editor is up; focus then rests on the tab,
  // not in the editor (which keeps the keys for typing).
  await expect(page.locator(".monaco-editor").first()).toBeVisible();
  await page.getByTestId("center-tab-oxplow:oxplow/file?path=aa.txt").click();

  await page.keyboard.press("Alt+ArrowDown");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=bb.txt")).toBeVisible();
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=aa.txt")).toBeHidden();
});

// A list's link column is live: Up and Down step through the page as it is now,
// not the rows it had when the page was opened.
test("Up and Down from a page opened from a page follow the page's rows as they change", async ({ page, daemon }) => {
  for (const title of ["Origin alpha", "Origin beta", "Origin gamma"]) await item(daemon, title);
  await page.goto("/");
  await openFromLauncher(page, "Item overview");
  await page.locator('[data-testid="page-file"]:visible').getByText("Origin alpha").click();
  await expect(page.getByTestId("page-nav-sibling-indicator").filter({ visible: true })).toContainText("1 of 3");

  // A row made after the page was opened is in the list the keys step. The
  // overview lists by title, so it comes between beta and gamma.
  const delta = await item(daemon, "Origin delta");
  await expect(page.getByTestId("page-nav-sibling-indicator").filter({ visible: true })).toContainText("1 of 4");
  const shown = page.locator('[data-testid^="center-tab-body-"]:visible');
  for (const title of ["beta", "delta", "gamma"]) {
    await page.keyboard.press("Alt+ArrowDown");
    await expect(shown).toContainText(`Origin ${title}`);
  }
  await page.getByTestId("page-nav-sibling-prev").filter({ visible: true }).click();
  await expect(shown).toContainText("Origin delta");

  // A page the page no longer lists has no Up or Down.
  await run(daemon, "oxplow.work_item.update", { uri: delta, title: "[archived] Origin delta" });
  await expect(page.getByTestId("page-nav-siblings").filter({ visible: true })).toHaveCount(0);
  await page.keyboard.press("Alt+ArrowDown");
  await expect(shown).toContainText("Origin delta");
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
  await openFromLauncher(page, "Suite Home");
  const tabs = page.locator('[data-testid^="center-tab-oxplow:oxplow/page?page=e2e/home"]');
  await expect(tabs).toHaveCount(1);
  await navigate(daemon, "oxplow:oxplow/page?page=e2e/home#mode=other", { new_tab: true });
  await expect(shownBody(page, "page=e2e/home#mode=other")).toHaveCount(1);
  await expect(tabs).toHaveCount(1);
  // The launcher finds it too.
  await openFromLauncher(page, "Suite Home");
  await expect(tabs).toHaveCount(1);
});

// A link is followed in the shown tab (Back returns), unless `replace`
// takes the shown page's place.
test("window.navigate pushes by default and replaces on request", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Suite Home");
  await expect(shownBody(page, "page=e2e/home")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();

  await navigate(daemon, "oxplow:oxplow/page?page=settings");
  await expect(shownBody(page, "page=settings")).toHaveCount(1);
  await expect(backButton(page)).toBeEnabled();

  await navigate(daemon, "oxplow:oxplow/page?page=e2e/second", { replace: true });
  await expect(shownBody(page, "page=e2e/second")).toHaveCount(1);
  // Settings was replaced: Back skips it, to Suite Home.
  await backButton(page).click();
  await expect(shownBody(page, "page=e2e/home")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();
});

// A file at a line is the file's view (`#lines=`): another line is another
// entry of the same tab, and Back returns to the first.
test("a file opened at a line lands at its #lines view, and Back returns to the line before", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "lines.txt"), Array.from({ length: 12 }, (_, i) => `line ${i + 1}`).join("\n") + "\n");
  await page.goto("/");
  // The window answers a navigate once its stream is shown: a page open is the sign.
  await openFromLauncher(page, "Suite Home");
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

// A tab mounts when it is first shown, so after a reload the tabs not shown
// yet are named by the title their entry kept, not by their page's slug.
test("after a reload, tabs not yet shown keep their titles and mount only when shown", async ({ page }) => {
  await page.goto("/");
  for (const query of ["Suite Home", "Suite Second", "Item overview"]) await openFromLauncher(page, query);
  const strip = page.locator('[data-testid^="center-tab-oxplow"]');
  const labels = () => strip.evaluateAll((els) => els.map((e) => (e.textContent ?? "").replace("×", "").trim()));
  await expect.poll(labels).toContain("Suite Second");
  const before = await labels();
  await page.reload();
  await expect(strip.first()).toBeVisible();
  await expect.poll(labels).toEqual(before);
  // Only the shown tab has a body.
  await expect(page.locator('[data-testid^="center-tab-body-"]')).toHaveCount(1);
  await strip.filter({ hasText: "Suite Second" }).click();
  await expect(page.locator('[data-testid^="center-tab-body-"]')).toHaveCount(2);
});
