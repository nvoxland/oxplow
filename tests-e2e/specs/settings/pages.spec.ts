import { readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Settings is one page whose view is a switch: an area is the section in
// view, each a link in the rail every area shares.
const AREAS: [string, string][] = [
  ["agents", "Agents"],
  ["lsp", "Language Servers"],
  ["data", "Data"],
  ["integrations", "Integrations"],
  ["ai", "AI"],
];

test("every Settings area renders, reached from the rail", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Every Setting");
  await expect(page.getByTestId("page-param-q").filter({ visible: true })).toBeVisible();
  for (const [id, title] of AREAS) {
    await page.getByTestId(`page-section-${id}`).filter({ visible: true }).click();
    await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText(title);
  }
  await page.getByTestId("page-section-data").filter({ visible: true }).click();
  await expect(page.locator('[data-testid="page-file"]:visible').getByRole("heading", { name: "Collectors" })).toBeVisible();
});

// An area is Settings at another view: one page, re-rendered, its area in
// the tab's address, and a step Back returns from.
test("a Settings area is the page's view: the same page, and Back returns to the one before", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  const body = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid*="page=settings"]:visible');
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=settings");
  // A mark on the page's own element (the rail inside it may redraw as
  // its layout settles; the page must not mount again).
  await body.getByTestId("page-file").evaluate((el) => {
    el.setAttribute("data-mounted-once", "yes");
  });
  await body.getByTestId("page-section-data").click();
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=settings#area=data");
  await expect(body.getByTestId("page-file")).toHaveAttribute("data-mounted-once", "yes");
  await page.getByTestId("page-nav-back").filter({ visible: true }).click();
  await expect(body).toHaveAttribute("data-testid", "center-tab-body-oxplow:oxplow/page?page=settings");
  await expect(body.getByTestId("page-nav-title")).toHaveText("Every Setting");
});

// The Agents area's prompt addition is an edit form over
// `oxplow.agent.set_prompt_append` (Save only when changed), written to
// project.yaml and kept across a reload.
test("a config edit is saved to project.yaml and read back", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings: Agents");
  const prompt = page.locator('[data-node="agents-prompt"]');
  const box = prompt.getByRole("textbox");
  const save = prompt.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await box.fill("Prefer small, reviewable commits.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect
    .poll(() => readFileSync(join(daemon.project, ".oxplow", "project.yaml"), "utf8"))
    .toContain("Prefer small, reviewable commits.");
  await page.reload();
  await openFromLauncher(page, "Settings: Agents");
  await expect(
    page.locator('[data-node="agents-prompt"]').getByRole("textbox"),
  ).toHaveValue("Prefer small, reviewable commits.");
});

// Stream Settings is the foundation's page over `v_stream`: its prompt
// is an edit form (Save only when changed), saved through the command, and
// kept across a reload.
test("a stream's prompt is edited and saved on its Settings page", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-testid^="navigator-strip-stream-"]').first().click({ button: "right" });
  await page.getByTestId("menu-item-stream.settings").click();
  const prompt = page.locator('[data-node="prompt"]:visible');
  // The prompt is markdown: the Markdown editor's surface.
  const box = prompt.locator('[contenteditable="true"]');
  const save = prompt.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await box.fill("Always answer in rhyme.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(page.locator('[data-node="prompt"]:visible [contenteditable="true"]')).toHaveText("Always answer in rhyme.");
});

// New Stream is the foundation's form over `oxplow.stream.create`: a source
// shows only its own fields, and the page gives way to the stream made. On
// a daemon of its own: a second stream is the window's stream afterwards, and
// a spec after this one on the worker's daemon would list its worktree, not
// the project's.
test("New Stream forks a branch into a stream and gives way to it", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "New Stream");
  const form = page.locator('[data-testid="viz-form"]:visible');
  await expect(form.getByTestId("viz-form-fields-existing_branch")).toBeVisible();
  await expect(form.getByTestId("viz-form-fields-new_branch")).toHaveCount(0);
  await form.getByTestId("viz-form-fields-source").selectOption("new");
  await expect(form.getByTestId("viz-form-fields-existing_branch")).toHaveCount(0);
  await form.getByTestId("viz-form-fields-title").fill("Side Work");
  await form.getByTestId("viz-form-fields-new_branch").fill("side-work");
  await form.getByRole("button", { name: "New Stream" }).click();
  await expect(page.locator('[data-testid^="navigator-strip-stream-"]')).toHaveCount(2);
  await expect(page.locator('[data-testid="viz-form"]:visible')).toHaveCount(0);
});
