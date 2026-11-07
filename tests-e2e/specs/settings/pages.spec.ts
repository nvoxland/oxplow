import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// Settings is one page whose view is a switch: an area is the section in
// view, each a link in the rail every area shares.
const AREAS: [string, string][] = [
  ["agents", "Agents"],
  ["lsp", "Language Servers"],
  ["data", "Data"],
  ["integrations", "Integrations"],
  ["ai", "AI"],
];

test("every Settings area renders, reached from the rail", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings");
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Every Setting");
  await expect(shown(page).getByTestId("page-param-q")).toBeVisible();
  for (const [id, title] of AREAS) {
    await page.getByTestId(`page-section-${id}`).filter({ visible: true }).click();
    await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText(title);
  }
  await shown(page).getByTestId("page-section-data").click();
  await expect(shown(page).getByTestId("page-file").getByRole("heading", { name: "Collectors" })).toBeVisible();
});

// New Stream is the foundation's form over `oxplow.stream.create`: a source
// shows only its own fields, and the page gives way to the stream made. On
// a daemon of its own: a second stream is the window's stream afterwards, and
// a spec after this one on the worker's daemon would list its worktree, not
// the project's.
test("New Stream forks a branch into a stream and gives way to it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=new-stream");
  const form = shown(page).getByTestId("viz-form");
  await expect(form.getByTestId("viz-form-fields-existing_branch")).toBeVisible();
  await expect(form.getByTestId("viz-form-fields-new_branch")).toHaveCount(0);
  await form.getByTestId("viz-form-fields-source").selectOption("new");
  await expect(form.getByTestId("viz-form-fields-existing_branch")).toHaveCount(0);
  await form.getByTestId("viz-form-fields-title").fill("Side Work");
  await form.getByTestId("viz-form-fields-new_branch").fill("side-work");
  await form.getByRole("button", { name: "New Stream" }).click();
  await expect(page.locator('[data-testid^="navigator-strip-stream-"]')).toHaveCount(2);
  // The new stream's own first page is a form too (New Session), so only this
  // form's own fields say it gave way.
  await expect(form.getByTestId("viz-form-fields-new_branch")).toHaveCount(0);
  await expect(form.getByTestId("viz-form-fields-source")).toHaveCount(0);
  await expect(form.getByRole("button", { name: "Start a new session" })).toBeVisible();
});
