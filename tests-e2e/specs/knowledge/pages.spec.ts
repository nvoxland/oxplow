import { readFileSync } from "node:fs";
import { join } from "node:path";

import { fileTask, run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, reload, shown, taskIdOf } from "../../support/ui.js";

// What a page lists, filters, writes and says is a fixture of
// oxplow-bundled (`knowledge-*`). These specs are what only the app's shell
// does: the markdown editor, its links, its toolbar, and the Save button.

const uriOf = (id: string) => `oxplow:oxplow/knowledge?id=${id}&store=oxwiki`;
const fileOf = (daemon: { project: string }, id: string) => join(daemon.project, ".oxplow", "oxwiki", `${id}.md`);

test("a written page renders, and a task link in it opens the task", async ({ page, daemon }) => {
  const created = await fileTask(daemon, "Wombat follow-up");
  await run(daemon, "oxwiki.page.write", {
    title: "Wombat plan",
    body: `# Wombat plan\n\nFirst do [[${taskIdOf(created.result.uri)}]].\n`,
  });
  await open(page, daemon, uriOf("wombat-plan"));
  const knowledge = page.locator('[data-testid="page-file"]:visible');
  await expect(knowledge).toContainText("First do");
  // The link shows the task's title (the body keeps its id).
  const link = knowledge.locator(`[data-ref-title][href="${created.result.uri}"]`);
  await expect(link).toHaveText("Wombat follow-up");
  await link.click();
  await expect(page.locator('[data-node="state"]')).toBeVisible();
  await expect(page.locator("body")).toContainText("Wombat follow-up");
});

// The index's filter is a row in the page's rail: its value opens it. What
// it lists is a fixture; the match marked in the title and the text is the
// shell's.
test("the Knowledge index marks the filter's match", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", {
    title: "Kiwi notes",
    body: "# Kiwi notes\n\nThe quokka sleeps all day.\n",
  });
  await run(daemon, "oxwiki.page.write", {
    title: "Other notes",
    body: "# Other notes\n\nNothing to see.\n",
  });
  await open(page, daemon, "oxplow:oxplow/page?page=knowledge");
  const index = page.locator('[data-section="pages"]:visible');
  await expect(index).toContainText("Kiwi notes");
  await page.getByTestId("page-param-filter-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-filter-edit").fill("quokka");
  await page.keyboard.press("Enter");
  await expect(index).toContainText("Kiwi notes");
  await expect(index).not.toContainText("Other notes");
  await expect(index.locator("mark.ox-mark")).toContainText("quokka");
});

// The page's body is the markdown editor (Save only when changed).
test("a knowledge page is edited in place and saved", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Ferret notes", body: "# Ferret notes\n\nThey steal socks.\n" });
  await open(page, daemon, uriOf("ferret-notes"));
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
  await reload(page);
  await expect(page.locator('[data-node="body"]:visible')).toContainText("Also keys.");
});

// The tab is named by the page's title, not its id.
test("a knowledge page's tab shows its title", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Pangolin facts", body: "# Pangolin facts\n\nScaly.\n" });
  await open(page, daemon, uriOf("pangolin-facts"));
  await expect(page.getByTestId(`center-tab-${uriOf("pangolin-facts")}`)).toContainText("Pangolin facts");
});

// In the editor a bare link shows its page's title, as the viewer does,
// while the body keeps the id.
test("a page link in the editor shows its page's title", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Otter den", body: "# Otter den\n" });
  await run(daemon, "oxwiki.page.write", { title: "Otter notes", body: "# Otter notes\n\nSee [[otter-den]] first.\n" });
  await open(page, daemon, uriOf("otter-notes"));
  const body = page.locator('[data-node="body"]:visible');
  const href = uriOf("otter-den");
  await expect(body.locator(`[data-ref-title][href="${href}"]`)).toHaveText("Otter den");
  await expect(body.locator(`a[href="${href}"]:not([data-ref-title])`)).toBeHidden();
});

// `[[` in the editor lists what it could link to; the picked page goes in
// as its link, shows its title, and saves as the wikilink.
test("[[ completes a link to a knowledge page", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Badger sett", body: "# Badger sett\n" });
  await run(daemon, "oxwiki.page.write", { title: "Badger notes", body: "# Badger notes\n\nStart here.\n" });
  // The completion lists what site search has indexed.
  await searchable(daemon, "badger", "knowledge");
  await open(page, daemon, uriOf("badger-notes"));
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("Start here.");
  await body.locator(".ProseMirror").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" See [[badger se");
  const href = uriOf("badger-sett");
  await expect(page.getByTestId(`ref-completion-row-${href}`)).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(shown(page).getByTestId("ref-completion")).toHaveCount(0);
  await expect(body.locator(`[data-ref-title][href="${href}"]`)).toHaveText("Badger sett");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect(save).toBeDisabled();
  // The page's file keeps the wikilink a person would have typed.
  await expect.poll(() => readFileSync(fileOf(daemon, "badger-notes"), "utf8")).toContain("See [[badger-sett]]");
  await reload(page);
  await expect(page.locator('[data-node="body"]:visible').locator(`[data-ref-title][href="${href}"]`)).toHaveText("Badger sett");
});

// A selection floats the format toolbar.
test("the editor formats a selection", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Heron notes", body: "# Heron notes\n\nStart here.\n" });
  await open(page, daemon, uriOf("heron-notes"));
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("Start here.");
  await body.locator(".ProseMirror p", { hasText: "Start here." }).click();
  await expect(body.locator(".ProseMirror")).toBeFocused();
  await page.keyboard.press("End");
  // A keyboard selection ("Home" is the document's start on macOS).
  // The editor learns of the browser's selection from `selectionchange`, a
  // task after the key. A key pressed before it has caught up with the last
  // one finds the editor's older selection written back over the browser's
  // (under load, two or more of the eleven presses were undone). So each
  // press waits until the editor (Tiptap leaves it on the element) holds
  // the selection the keys so far made, as a person's slower keys do.
  const held = () =>
    body.locator(".ProseMirror").evaluate((el) => {
      const { from, to } = (el as unknown as { editor: { state: { selection: { from: number; to: number } } } }).editor.state.selection;
      return to - from;
    });
  for (let i = 1; i <= "Start here.".length; i++) {
    await page.keyboard.press("Shift+ArrowLeft");
    await expect.poll(held).toBe(i);
  }
  await page.getByTestId("selection-toolbar-bold").click();
  await expect(body.locator(".ProseMirror p strong")).toHaveText("Start here.");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect.poll(() => readFileSync(fileOf(daemon, "heron-notes"), "utf8")).toMatch(/\*\*Start here\.\*\*/);
  expect(readFileSync(fileOf(daemon, "heron-notes"), "utf8")).toMatch(/^# Heron notes\n/);
});

// The Source view shows the page as stored, wikilinks and all; an edit
// there saves like any other.
test("a knowledge page's Source view edits the stored text", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", { title: "Stoat den", body: "# Stoat den\n" });
  await run(daemon, "oxwiki.page.write", { title: "Stoat notes", body: "# Stoat notes\n\nSee [[stoat-den]].\n" });
  await open(page, daemon, uriOf("stoat-notes"));
  const body = page.locator('[data-node="body"]:visible');
  await expect(body).toContainText("See");
  await body.getByTestId("markdown-source-toggle").click();
  const source = body.getByTestId("markdown-source");
  await expect(source).toHaveValue(/See \[\[stoat-den\]\]\./);
  await source.fill("# Stoat notes\n\nSee [[stoat-den]] and **more**.\n");
  const save = body.getByRole("button", { name: "Save" });
  await save.click();
  await expect.poll(() => readFileSync(fileOf(daemon, "stoat-notes"), "utf8")).toContain("See [[stoat-den]] and **more**.");
  await body.getByTestId("markdown-source-toggle").click();
  await expect(body.locator(".ProseMirror strong")).toHaveText("more");
});

// The New Knowledge Page form is a page of the app: a title and a markdown
// body (the rich text editor), no id. Submitting writes the page and the
// form gives way to it.
test("New page on the Knowledge index writes a page and opens it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=knowledge");
  await shown(page).getByRole("button", { name: /New page/ }).click();
  // The body is markdown: the rich text editor. Enter there is a new line,
  // so the form is submitted from the title.
  await shown(page).locator('[contenteditable="true"]').first().fill("They like the sun.");
  const title = shown(page).getByTestId("viz-form-fields-title");
  await title.fill("Lemur facts");
  await title.press("Enter");
  await expect(page.getByTestId(`center-tab-${uriOf("lemur-facts")}`)).toBeVisible();
  await expect(page.locator('[data-node="body"]:visible')).toContainText("They like the sun.");
});
