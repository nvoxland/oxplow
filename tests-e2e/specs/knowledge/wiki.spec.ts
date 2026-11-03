import { readFileSync } from "node:fs";
import { join } from "node:path";

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
  const wiki = page.locator('[data-testid="page-file"]:visible');
  await expect(wiki).toContainText("First do");
  // The link shows the task's title (the body keeps its id).
  const link = wiki.locator(`[data-ref-title][href="${created.result.uri}"]`);
  await expect(link).toHaveText("Wombat follow-up");
  await link.click();
  await expect(page.locator('[data-node="state"]')).toBeVisible();
  await expect(page.locator("body")).toContainText("Wombat follow-up");
});

// The wiki index is a page: its filter matches title or body, and the
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
  await page.getByTestId("page-param-filter-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-filter-edit").fill("quokka");
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
  const body = page.locator('[data-node="body"]:visible');
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
  await expect(page.locator('[data-node="body"]:visible')).toContainText("Also keys.");
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
  const body = page.locator('[data-node="body"]:visible');
  const href = "oxplow:oxplow/wiki?slug=otter-den";
  await expect(body.locator(`[data-ref-title][href="${href}"]`)).toHaveText("Otter den");
  await expect(body.locator(`a[href="${href}"]:not([data-ref-title])`)).toBeHidden();
});

// `[[` in the editor lists what it could link to; the picked page goes in
// as its link, shows its title, and saves as the wikilink.
test("[[ completes a link to a wiki page", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "badger-sett", title: "Badger sett", body: "# Badger sett\n" });
  await run(daemon, "oxplow.knowledge.write_page", { slug: "badger-notes", title: "Badger notes", body: "# Badger notes\n\nStart here.\n" });
  await searchable(daemon, "badger", "wiki");
  await page.goto("/");
  await searchLauncher(page, "badger notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=badger-notes").click();
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("Start here.");
  await body.locator(".ProseMirror").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" See [[badger se");
  const href = "oxplow:oxplow/wiki?slug=badger-sett";
  await expect(page.getByTestId(`ref-completion-row-${href}`)).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("ref-completion")).toHaveCount(0);
  await expect(body.locator(`[data-ref-title][href="${href}"]`)).toHaveText("Badger sett");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect(save).toBeDisabled();
  // The page's file keeps the wikilink a person would have typed.
  await expect.poll(() => readFileSync(join(daemon.project, ".oxplow", "wiki", "badger-notes.md"), "utf8")).toContain("See [[badger-sett]]");
  await page.reload();
  await expect(page.locator('[data-node="body"]:visible').locator(`[data-ref-title][href="${href}"]`)).toHaveText("Badger sett");
});

// A selection floats the format toolbar.
test("the editor formats a selection", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "heron-notes", title: "Heron notes", body: "# Heron notes\n\nStart here.\n" });
  await searchable(daemon, "heron", "wiki");
  await page.goto("/");
  await searchLauncher(page, "heron notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=heron-notes").click();
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("Start here.");
  await body.locator(".ProseMirror p", { hasText: "Start here." }).click();
  await expect(body.locator(".ProseMirror")).toBeFocused();
  await page.keyboard.press("End");
  // A keyboard selection ("Home" is the document's start on macOS).
  for (let i = 0; i < "Start here.".length; i++) await page.keyboard.press("Shift+ArrowLeft");
  await page.getByTestId("selection-toolbar-bold").click();
  await expect(body.locator(".ProseMirror p strong")).toHaveText("Start here.");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect.poll(() => readFileSync(join(daemon.project, ".oxplow", "wiki", "heron-notes.md"), "utf8")).toMatch(/\*\*Start here\.\*\*/);
  expect(readFileSync(join(daemon.project, ".oxplow", "wiki", "heron-notes.md"), "utf8")).toMatch(/^# Heron notes\n/);
});

// The Source view shows the page as stored, wikilinks and all; an edit
// there saves like any other.
test("a wiki page's Source view edits the stored text", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "stoat-den", title: "Stoat den", body: "# Stoat den\n" });
  await run(daemon, "oxplow.knowledge.write_page", { slug: "stoat-notes", title: "Stoat notes", body: "# Stoat notes\n\nSee [[stoat-den]].\n" });
  await searchable(daemon, "stoat", "wiki");
  await page.goto("/");
  await searchLauncher(page, "stoat notes");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=stoat-notes").click();
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("See");
  await body.getByTestId("markdown-source-toggle").click();
  const source = body.getByTestId("markdown-source");
  await expect(source).toHaveValue(/See \[\[stoat-den\]\]\./);
  await source.fill("# Stoat notes\n\nSee [[stoat-den]] and **more**.\n");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect.poll(() => readFileSync(join(daemon.project, ".oxplow", "wiki", "stoat-notes.md"), "utf8")).toContain("See [[stoat-den]] and **more**.");
  await body.getByTestId("markdown-source-toggle").click();
  await expect(body.locator(".ProseMirror strong")).toHaveText("more");
});
