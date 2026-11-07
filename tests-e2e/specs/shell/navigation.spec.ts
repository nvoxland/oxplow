import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { navigate, open, shown, load, reload } from "../../support/ui.js";

const overviewTitle = (page: import("@playwright/test").Page) => page.getByTestId("page-nav-title").filter({ visible: true });

/// An item the test extension's overview lists, each title a link to its page.
async function item(daemon: Parameters<typeof run>[0] & { thread: string }, title: string) {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", { title, thread: daemon.thread });
  return created.result.uri;
}

async function openOverview(page: import("@playwright/test").Page, daemon: Parameters<typeof run>[0], title: string) {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/overview");
  await expect(shown(page).getByTestId("page-file")).toContainText(title);
  await expect(overviewTitle(page)).toHaveText("Item overview");
}

async function followItem(page: import("@playwright/test").Page, title: string) {
  await shown(page).getByTestId("page-file").getByText(title).click();
  await expect(overviewTitle(page)).toHaveText("Item");
}

test("Cmd+[ goes back through the tab's pages and Cmd+] goes forward", async ({ page, daemon }) => {
  await item(daemon, "Kestrel follow-up");
  await load(page);
  await openOverview(page, daemon, "Kestrel follow-up");
  await followItem(page, "Kestrel follow-up");

  await page.keyboard.press("Meta+[");
  await expect(overviewTitle(page)).toHaveText("Item overview");

  await page.keyboard.press("Meta+]");
  await expect(overviewTitle(page)).toHaveText("Item");
});

test("the active tab and its history survive a reload", async ({ page, daemon }) => {
  await item(daemon, "Osprey follow-up");
  await load(page);
  await openOverview(page, daemon, "Osprey follow-up");
  await followItem(page, "Osprey follow-up");

  await reload(page);
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
  await open(page, daemon, "oxplow:oxplow/page?page=e2e-tree/heron");
  await shown(page).getByTestId("page-file").getByRole("button", { name: "aa.txt" }).filter({ visible: true }).click();
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
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/overview");
  await shown(page).getByTestId("page-file").getByText("Origin alpha").click();
  await expect(page.getByTestId("page-nav-sibling-indicator").filter({ visible: true })).toContainText("1 of 3");

  // A row made after the page was opened is in the list the keys step. The
  // overview lists by title, so it comes between beta and gamma.
  const delta = await item(daemon, "Origin delta");
  await expect(page.getByTestId("page-nav-sibling-indicator").filter({ visible: true })).toContainText("1 of 4");
  const body = shown(page);
  for (const title of ["beta", "delta", "gamma"]) {
    await page.keyboard.press("Alt+ArrowDown");
    await expect(body).toContainText(`Origin ${title}`);
  }
  await page.getByTestId("page-nav-sibling-prev").filter({ visible: true }).click();
  await expect(body).toContainText("Origin delta");

  // A page the page no longer lists has no Up or Down.
  await run(daemon, "oxplow.work_item.update", { uri: delta, title: "[archived] Origin delta" });
  await expect(page.getByTestId("page-nav-siblings").filter({ visible: true })).not.toBeVisible();
  await page.keyboard.press("Alt+ArrowDown");
  await expect(body).toContainText("Origin delta");
});

const backButton = (page: import("@playwright/test").Page) => page.getByTestId("page-nav-back").filter({ visible: true });

// The same page in another view is the same tab: navigating to it with a
// new filter focuses the tab that has it, and never opens a second.
test("opening a page again with another filter focuses its tab", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  const tabs = page.locator('[data-testid^="center-tab-oxplow:oxplow/page?page=e2e/home"]');
  await expect(tabs).toHaveCount(1);
  await navigate(page, daemon, "oxplow:oxplow/page?page=e2e/home#mode=other", { new_tab: true });
  await expect(shown(page, "page=e2e/home#mode=other")).toHaveCount(1);
  await expect(tabs).toHaveCount(1);
  // The launcher finds it too.
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await expect(tabs).toHaveCount(1);
});

// A link is followed in the shown tab (Back returns), unless `replace`
// takes the shown page's place.
test("window.navigate pushes by default and replaces on request", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await expect(shown(page, "page=e2e/home")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();

  await navigate(page, daemon, "oxplow:oxplow/page?page=settings");
  await expect(shown(page, "page=settings")).toHaveCount(1);
  await expect(backButton(page)).toBeEnabled();

  await navigate(page, daemon, "oxplow:oxplow/page?page=e2e/second", { replace: true });
  await expect(shown(page, "page=e2e/second")).toHaveCount(1);
  // Settings was replaced: Back skips it, to Suite Home.
  await backButton(page).click();
  await expect(shown(page, "page=e2e/home")).toHaveCount(1);
  await expect(backButton(page)).toBeDisabled();
});

// A file at a line is the file's view (`#lines=`): another line is another
// entry of the same tab, and Back returns to the first.
test("a file opened at a line lands at its #lines view, and Back returns to the line before", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "lines.txt"), Array.from({ length: 12 }, (_, i) => `line ${i + 1}`).join("\n") + "\n");
  await load(page);
  // The window answers a navigate once its stream is shown: a page open is the sign.
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  await navigate(page, daemon, "oxplow:oxplow/file?path=lines.txt#lines=3-3");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=3-3")).toBeVisible();
  await expect(page.locator(".monaco-editor").first()).toBeVisible();

  await navigate(page, daemon, "oxplow:oxplow/file?path=lines.txt#lines=8-8");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=8-8")).toBeVisible();
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/file?path=lines.txt"]')).toHaveCount(1);

  await backButton(page).click();
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=lines.txt#lines=3-3")).toBeVisible();
});

// A tab mounts when it is first shown, so after a reload the tabs not shown
// yet are named by the title their entry kept, not by their page's slug.
test("after a reload, tabs not yet shown keep their titles and mount only when shown", async ({ page, daemon }) => {
  for (const id of ["home", "second", "overview"]) await open(page, daemon, `oxplow:oxplow/page?page=e2e/${id}`, { new_tab: true });
  const strip = page.locator('[data-testid^="center-tab-oxplow"]');
  const labels = () => strip.evaluateAll((els) => els.map((e) => (e.textContent ?? "").replace("×", "").trim()));
  await expect.poll(labels).toContain("Suite Second");
  const before = await labels();
  await reload(page);
  await expect(strip.first()).toBeVisible();
  await expect.poll(labels).toEqual(before);
  // Only the shown tab has a body.
  await expect(page.locator('[data-testid^="center-tab-body-"]')).toHaveCount(1);
  await strip.filter({ hasText: "Suite Second" }).click();
  await expect(page.locator('[data-testid^="center-tab-body-"]')).toHaveCount(2);
});
