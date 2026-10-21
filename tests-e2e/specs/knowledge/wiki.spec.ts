import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher, workItemIdOf } from "../../support/ui.js";

test("a written page renders, and a task link in it opens the task", async ({ page, daemon }) => {
  const created = await run<{ result: { ref: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat follow-up",
    thread: daemon.threadUri,
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "wombat-plan",
    title: "Wombat plan",
    body: `# Wombat plan\n\nFirst do [[${workItemIdOf(created.result.ref)}]].\n`,
  });
  await searchable(daemon, "wombat", "wiki");
  await page.goto("/");
  await searchLauncher(page, "wombat plan");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=wombat-plan").click();
  const wiki = page.getByTestId("page-wiki");
  await expect(wiki).toContainText("First do");
  await wiki.locator(`a[href="${created.result.ref}"]`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
  await expect(page.locator("body")).toContainText("Wombat follow-up");
});

// The wiki index is a lens page: its filter matches title or body, and the
// matched text is marked in the title and the snippet.
test("the wiki index filters by body and marks the match", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "kiwi-notes",
    title: "Kiwi notes",
    body: "# Kiwi notes\n\nThe quokka sleeps all day.\n",
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "other-notes",
    title: "Other notes",
    body: "# Other notes\n\nNothing to see.\n",
  });
  await page.goto("/");
  await openFromLauncher(page, "Wiki");
  const index = page.locator('[data-section="pages"]:visible');
  await expect(index).toContainText("Kiwi notes");
  // The search is a row in the page's rail: its value opens it.
  await page.getByTestId("lens-param-filter-value").filter({ visible: true }).click();
  await page.getByTestId("lens-param-filter-edit").fill("quokka");
  await page.keyboard.press("Enter");
  await expect(index).toContainText("Kiwi notes");
  await expect(index).not.toContainText("Other notes");
  await expect(index.locator("mark.ox-mark")).toContainText("quokka");
});
