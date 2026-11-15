import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

test("Settings links to category forms and operational tools", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings");
  await expect(shown(page).getByTestId("page-param-q")).toBeVisible();
  await expect(shown(page).getByTestId("settings-viz")).toHaveCount(0);
  await open(page, daemon, "oxplow:oxplow/page?page=settings-category#category=snapshots");
  const settings = shown(page).getByTestId("settings-viz");
  await expect(settings).toBeVisible();
  await expect(settings.getByLabel(/View\/edit level/).first()).toBeVisible();
  await expect(settings.getByText("All levels").first()).toBeVisible();
  await open(page, daemon, "oxplow:oxplow/page?page=management#area=data");
  await expect(shown(page).getByRole("heading", { name: "Collectors", exact: true })).toBeVisible();
});

test("a hidden Global override can be edited and inherited after Project reset", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=settings-category#category=snapshots&setting=oxplow.snapshots.retentionDays");
  const setting = shown(page).getByTestId("settings-viz").locator("section").first();
  await setting.getByRole("button", { name: "Edit project value", exact: true }).click();
  await setting.getByRole("spinbutton").fill("17");
  await setting.getByRole("button", { name: "Save", exact: true }).click();
  await expect(setting.getByText(/Source: project/)).toBeVisible();
  await setting.getByRole("combobox").selectOption("global");
  await setting.getByRole("button", { name: "Edit global value", exact: true }).click();
  await setting.getByRole("spinbutton").fill("29");
  await setting.getByRole("button", { name: "Save", exact: true }).click();
  await expect(setting.getByText(/Source: project/)).toBeVisible();
  await setting.getByText("All levels", { exact: true }).click();
  await expect(setting.getByRole("row").filter({ hasText: "global" })).toContainText("29");
  await setting.getByRole("combobox").selectOption("project");
  await setting.getByRole("button", { name: "Edit project value", exact: true }).click();
  await setting.getByRole("button", { name: "Reset to inherited value", exact: true }).click();
  await setting.getByRole("button", { name: "Save", exact: true }).click();
  await expect(setting.getByText(/Source: global/)).toBeVisible();
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
