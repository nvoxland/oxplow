import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, workItemIdOf } from "../../support/ui.js";

test("an extension's ref renders as a link and opens its page", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Platypus item");
  // `[[item:<n>]]` is the test extension's wikilink for `oxplow:e2e/item?id=%3Cn%3E`.
  writeFileSync(join(daemon.project, "platypus-notes.md"), `# Platypus notes\n\nThe item is [[item:${n}]].\n`);
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=platypus-notes.md#oxplow.renderer=oxplow-foundation/markdown");
  await page.locator('[data-testid="page-file"]:visible').locator(`a[href="oxplow:e2e/item?id=${n}"]`).click();
  // The link shows the item's title; its page is a table over `v_e2e_item`.
  await expect(page.locator('[data-testid="page-file"]:visible').getByTestId("viz-row-0")).toContainText("Platypus item");
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
  await openFromLauncher(page, "Suite Home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
}

test("an extension's resource URI opens on its renderer, and the tab's id is the URI", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Quokka item");
  await ready(page);
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(daemon, uri);
  await expect(page.getByTestId(`center-tab-${uri}`)).toBeVisible();
  await expect(page.locator('[data-testid="page-file"]:visible').getByTestId("viz-row-0")).toContainText("Quokka item");
});

test("a URI with two renderers is opened with the other one through Open with", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Wombat item");
  await ready(page);
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(daemon, uri);
  await expect(page.locator('[data-testid="page-file"]:visible').getByTestId("viz-row-0")).toContainText("Wombat item");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await expect(page.getByTestId("menu-item-page-open-with-e2e/item")).toContainText("✓");
  await page.getByTestId("menu-item-page-open-with-e2e/item-raw").click();
  await expect(page.getByTestId(`center-tab-${uri}#oxplow.renderer=e2e/item-raw`)).toBeVisible();
  await expect(page.locator('[data-testid="page-file"]:visible')).toContainText(uri);
  await expect(page.locator('[data-testid="page-file"]:visible')).not.toContainText("Wombat item");
});

test("a URI nothing renders shows the fallback page, not an error", async ({ page, daemon }) => {
  await ready(page);
  await navigate(daemon, "oxplow:e2e/undeclared?id=1");
  await expect(page.getByTestId("page-resource")).toBeVisible();
  await expect(page.getByTestId("resource-dead-link")).toBeVisible();
  await expect(page.getByTestId("undo-toast")).toHaveCount(0);
});
