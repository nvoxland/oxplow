import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, searchLauncher, workItemIdOf } from "../../support/ui.js";

test("a written page renders, and a task link in it opens the task", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat follow-up",
    thread: daemon.thread,
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "wombat-plan",
    title: "Wombat plan",
    body: `# Wombat plan\n\nFirst do [[${workItemIdOf(created.result.uri)}]].\n`,
  });
  await searchable(daemon, "wombat", "wiki");
  await page.goto("/");
  await searchLauncher(page, "wombat plan");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=wombat-plan").click();
  const wiki = page.locator('[data-testid="page-lens"]:visible');
  await expect(wiki).toContainText("First do");
  // The link shows the task's title (the body keeps its id).
  const link = wiki.locator(`[data-ref-title][href="${created.result.uri}"]`);
  await expect(link).toHaveText("Wombat follow-up");
  await link.click();
  await expect(page.getByTestId("lens-part-state")).toBeVisible();
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

// A wiki page is the foundation's `wiki-page` document: its body is the
// markdown editor (Save only when changed), and a page not yet written is
// made by its first Save.
test("a wiki page is edited in place and saved, and a new page is made by its first Save", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "ferret-notes", title: "Ferret notes", body: "# Ferret notes\n\nThey steal socks.\n" });
  await searchable(daemon, "ferret", "wiki");
  await page.goto("/");
  await searchLauncher(page, "ferret notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=ferret-notes").click();
  const body = page.locator('[data-testid="lens-part-body"]:visible');
  const save = body.getByRole("button", { name: "Save" });
  await expect(body).toContainText("They steal socks.");
  await expect(save).toBeDisabled();
  await body.locator(".ProseMirror").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" Also keys.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(page.locator('[data-testid="lens-part-body"]:visible')).toContainText("Also keys.");
});

// In the editor a bare link shows its page's title, as the viewer does,
// while the body keeps the slug.
test("a wiki link in the editor shows its page's title", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "otter-den", title: "Otter den", body: "# Otter den\n" });
  await run(daemon, "oxplow.knowledge.write_page", { slug: "otter-notes", title: "Otter notes", body: "# Otter notes\n\nSee [[otter-den]] first.\n" });
  await searchable(daemon, "otter", "wiki");
  await page.goto("/");
  await searchLauncher(page, "otter notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=otter-notes").click();
  const body = page.locator('[data-testid="lens-part-body"]:visible');
  const href = "oxplow:oxplow/wiki?slug=otter-den";
  await expect(body.locator(`[data-ref-title][href="${href}"]`)).toHaveText("Otter den");
  await expect(body.locator(`a[href="${href}"]:not([data-ref-title])`)).toBeHidden();
});
