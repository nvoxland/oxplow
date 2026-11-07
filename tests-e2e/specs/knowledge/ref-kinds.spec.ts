import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { navigate, workItemIdOf, shown } from "../../support/ui.js";

test("an extension's ref renders as a link and opens its page", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Platypus item");
  // `[[item:<n>]]` is the test extension's wikilink for `oxplow:e2e/item?id=%3Cn%3E`.
  writeFileSync(join(daemon.project, "platypus-notes.md"), `# Platypus notes\n\nThe item is [[item:${n}]].\n`);
  await navigate(page, daemon, "oxplow:oxplow/file?path=platypus-notes.md#oxplow.renderer=oxplow-foundation/markdown");
  await shown(page).getByTestId("page-file").locator(`a[href="oxplow:e2e/item?id=${n}"]`).click();
  // The link shows the item's title; its page is a table over `v_e2e_item`.
  await expect(shown(page).getByTestId("page-file").getByTestId("viz-row-0")).toContainText("Platypus item");
});

async function createItem(daemon: Parameters<typeof run>[0] & { thread: string }, title: string) {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title,
    thread: daemon.thread,
  });
  return workItemIdOf(created.result.uri).replace(/^tsk/, "");
}

test("an extension's resource URI opens on its renderer, and the tab's id is the URI", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Quokka item");
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(page, daemon, uri);
  await expect(page.getByTestId(`center-tab-${uri}`)).toBeVisible();
  await expect(shown(page).getByTestId("page-file").getByTestId("viz-row-0")).toContainText("Quokka item");
});

test("a URI with two renderers is opened with the other one through Open with", async ({ page, daemon }) => {
  const n = await createItem(daemon, "Wombat item");
  const uri = `oxplow:e2e/item?id=${n}`;
  await navigate(page, daemon, uri);
  await expect(shown(page).getByTestId("page-file").getByTestId("viz-row-0")).toContainText("Wombat item");
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-open-with").click();
  await expect(page.getByTestId("menu-item-page-open-with-e2e/item")).toContainText("✓");
  await page.getByTestId("menu-item-page-open-with-e2e/item-raw").click();
  await expect(page.getByTestId(`center-tab-${uri}#oxplow.renderer=e2e/item-raw`)).toBeVisible();
  await expect(shown(page).getByTestId("page-file")).toContainText(uri);
  await expect(shown(page).getByTestId("page-file")).not.toContainText("Wombat item");
});

test("a URI nothing renders shows the fallback page, not an error", async ({ page, daemon }) => {
  await navigate(page, daemon, "oxplow:e2e/undeclared?id=1");
  await expect(page.getByTestId("page-resource")).toBeVisible();
  await expect(page.getByTestId("resource-dead-link")).toBeVisible();
  await expect(page.getByTestId("undo-toast")).not.toBeVisible();
});
