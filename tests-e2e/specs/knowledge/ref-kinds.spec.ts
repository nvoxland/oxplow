import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("an extension's ref renders as a link and opens its page", async ({ page, daemon }) => {
  const created = await run<{ result: { id: string } }>(daemon, "work_item.create", {
    title: "Platypus item",
    thread: daemon.thread,
  });
  const n = created.result.id.replace(/^tsk/, "");
  // `[[item:<n>]]` is the test extension's wikilink for `e2e_item:<n>`.
  await run(daemon, "knowledge.write_page", {
    slug: "platypus-notes",
    title: "Platypus notes",
    body: `# Platypus notes\n\nThe item is [[item:${n}]].\n`,
  });
  await searchable(daemon, "platypus", "wiki");
  await page.goto("/");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("platypus notes");
  await page.getByTestId("launcher-hit-wiki:platypus-notes").click();
  await page.getByTestId("page-wiki").locator(`a[href="e2e_item:${n}"]`).click();
  // The extension's `item` page: its lens over `v_e2e_item`.
  await expect(page.getByTestId("page-lens").getByTestId("lens-row-0")).toContainText("Platypus item");
});
