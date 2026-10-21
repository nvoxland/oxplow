import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher, workItemIdOf } from "../../support/ui.js";

test("an extension's ref renders as a link and opens its page", async ({ page, daemon }) => {
  const created = await run<{ result: { ref: string } }>(daemon, "oxplow.work_item.create", {
    title: "Platypus item",
    thread: daemon.threadUri,
  });
  const n = workItemIdOf(created.result.ref).replace(/^tsk/, "");
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
  await page.getByTestId("page-wiki").locator(`a[href="oxplow:e2e/item?id=${n}"]`).click();
  // The extension's `item` page: its lens over `v_e2e_item`.
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Platypus item");
});
