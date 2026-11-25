import { expect, test } from "../../support/fixtures.js";
import { installExtension } from "../../support/pages.js";
import { run } from "../../support/daemon.js";
import { open, reload, shown } from "../../support/ui.js";

const source = `title: Results
params: [{ name: q }]
loaders:
  results:
    sql: SELECT '2026-01-01' AS day, :q AS label, 7 AS total, 3 AS tax
view: { stack: [{ id: results, title: Results, viz: table, chartable: true, from: results }] }
`;

test("a page table can change presentation and keep a live saved view", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-show-as", { files: { "pages/results.yaml": source } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_show_as/results&q=Selected");
  const results = shown(page).getByTestId("node-results");
  const showAs = results.getByRole("combobox", { name: "Show as", exact: true });
  await expect(results).toContainText("Selected");
  await expect(showAs.locator("option")).toHaveText(["Table", "List", "Number", "Bar", "Line", "Treemap"]);
  await showAs.selectOption("line");
  await expect(results.getByRole("combobox", { name: "X axis", exact: true })).toHaveValue("day");
  await expect(results.getByRole("combobox", { name: "Y axis", exact: true })).toHaveValue("total");
  await results.getByRole("combobox", { name: "Series", exact: true }).selectOption("label");
  await expect(results.getByTestId("viz-line-series")).toContainText("Selected");
  await reload(page);
  await expect(results.getByTestId("viz-line-series")).toContainText("Selected");

  await results.getByRole("button", { name: "Save as Page" }).click();
  const title = results.getByTestId("save-view-title");
  await title.fill("Saved Result Curve");
  await title.press("Enter");
  const saved = shown(page, "my_pages/saved-result-curve");
  await expect(saved.getByTestId("viz-line-series")).toContainText("Selected");
});

test("a temporary table saves the selected value with its original loader", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  const { result } = await run<{ result: { uri: string; opened: boolean } }>(daemon, "oxplow.page.show", {
    spec: source.replace("name: q", "name: q, default: Temporary"),
  });
  expect(result.opened).toBe(true);
  await page.getByTestId(`center-tab-${result.uri}`).click();
  const table = shown(page, result.uri).getByTestId("node-results");
  await expect(table).toContainText("Temporary");
  await table.getByRole("combobox", { name: "Show as", exact: true }).selectOption("number");
  await table.getByRole("combobox", { name: "Value", exact: true }).selectOption("tax");
  await expect(table.getByTestId("viz-number")).toHaveText("3");
  await table.getByRole("button", { name: "Save as Page" }).click();
  const title = table.getByTestId("save-view-title");
  await title.fill("Saved Temporary Number");
  await title.press("Enter");
  await expect(shown(page, "my_pages/saved-temporary-number").getByTestId("viz-number")).toHaveText("3");
});
