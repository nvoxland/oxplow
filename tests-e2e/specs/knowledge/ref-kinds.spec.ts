import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher, workItemIdOf } from "../../support/ui.js";

test("an extension's ref renders as a link and opens its page", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Platypus item",
    thread: daemon.thread,
  });
  const n = workItemIdOf(created.result.uri).replace(/^tsk/, "");
  // `[[item:<n>]]` is the test extension's wikilink for `oxplow:e2e/item?id=%3Cn%3E`.
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "platypus-notes",
    title: "Platypus notes",
    body: `# Platypus notes\n\nThe item is [[item:${n}]].\n`,
  });
  await searchable(daemon, "platypus", "wiki");
  await page.goto("/");
  await searchLauncher(page, "platypus notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=platypus-notes").click();
  await page.locator('[data-testid="page-lens"]:visible').locator(`a[href="oxplow:e2e/item?id=${n}"]`).click();
  // The extension's `item` page: its lens over `v_e2e_item`.
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Platypus item");
});

const navigate = (daemon: Parameters<typeof run>[0], uri: string) => run(daemon, "oxplow.window.navigate", { uri });

async function createItem(daemon: Parameters<typeof run>[0] & { thread: string }, title: string) {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title,
    thread: daemon.thread,
  });
  return workItemIdOf(created.result.uri).replace(/^tsk/, "");
}

/// The window answers a navigate once its stream is shown: a page open is the sign.
async function ready(page: import("@playwright/test").Page) {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
}

test("an extension's resource URI opens on its renderer, and the tab's id is the URI", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Quokka item");
  await ready(page);
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(daemon, uri);
  await expect(page.getByTestId(`center-tab-${uri}`)).toBeVisible();
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Quokka item");
});

test("a URI with two renderers is opened with the other one through Open with", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Wombat item");
  await ready(page);
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(daemon, uri);
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Wombat item");
  const openWith = page.getByTestId("page-nav-open-with").filter({ visible: true });
  await expect(openWith).toBeVisible();
  await openWith.selectOption("e2e/item-raw");
  await expect(page.getByTestId(`center-tab-${uri}#oxplow.renderer=e2e/item-raw`)).toBeVisible();
  await expect(page.getByTestId("page-lens").filter({ visible: true })).toContainText(uri);
  await expect(page.getByTestId("page-lens").filter({ visible: true })).not.toContainText("Wombat item");
});

test("a URI nothing renders shows the fallback page, not an error", async ({ page, daemon }) => {
  await ready(page);
  await navigate(daemon, "oxplow:e2e/undeclared?id=1");
  await expect(page.getByTestId("page-resource")).toBeVisible();
  await expect(page.getByTestId("resource-dead-link")).toBeVisible();
  await expect(page.getByTestId("undo-toast")).toHaveCount(0);
});

test("an https link opens a tab whose id is the URL", async ({ page, daemon }) => {
  await ready(page);
  await navigate(daemon, "https://example.com/docs");
  await expect(page.getByTestId("center-tab-https://example.com/docs")).toBeVisible();
  await expect(page.getByTestId("page-external-url")).toBeVisible();
});

test("a file URI with a rev opens the read-only viewer, and without one the editor", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "koala.txt"), "koala\n");
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  git("add", "koala.txt");
  git("commit", "-q", "-m", "Add koala");
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=koala.txt&rev=git:HEAD");
  // The foundation's `file-viewer` lens: a `code` viz of the file at the revision.
  await expect(page.getByTestId("lens-code")).toContainText("koala");
  await navigate(daemon, "oxplow:oxplow/file?path=koala.txt");
  await expect(page.getByTestId("page-file")).toBeVisible();
  await expect(page.getByTestId("page-file").locator(".monaco-editor").first()).toBeVisible();
});
