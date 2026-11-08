import type { Page } from "@playwright/test";

import type { Daemon } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown, reload } from "../../support/ui.js";

/// A page of the browser suite's own: its rows stand in for commits, and its
/// filters are the kinds a page declares (a date range with a custom end, a
/// search, a bool), with `commit` a row's pick that the next section reads.
const history = `title: Params History
category: Data
params:
- name: range
  role: filter
  label: When
  type: date_range
  default: 30d
  presets: [7d, 30d, 90d, all]
- name: q
  role: filter
  label: Search subjects
  type: search
- name: merges
  role: filter
  label: Merges only
  type: bool
  default: false
- name: commit
  label: Commit
loaders:
  commits:
    sql: |
      SELECT sha, subject, parents FROM (
        SELECT 'c1' AS sha, 'Add the numbat' AS subject, 1 AS parents
        UNION ALL SELECT 'c2', 'Add the wombat', 1)
      WHERE (:q IS NULL OR subject LIKE '%' || :q || '%')
        AND (:merges = 0 OR parents > 1)
      ORDER BY sha
  files:
    sql: SELECT 'file of ' || :commit AS path WHERE :commit IS NOT NULL
view:
  stack:
  - id: commits
    title: Commits
    from: commits
    viz: table
    columns:
    - { key: sha, label: Commit, select: commit }
    - { key: subject, label: Subject, highlight: q }
    empty: No commits match.
  - id: files
    title: Files in the commit
    from: files
    viz: table
    columns: [{ key: path, label: File }]
    empty: Click a commit to see its files.
`;

const ROUTE = "oxplow:oxplow/page?page=e2e_params/history";

async function openHistory(page: Page, daemon: Daemon) {
  await installExtension(daemon, "e2e-params", { files: { "pages/history.yaml": history } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_params/history");
  const host = shown(page).getByTestId("page-host");
  return {
    host,
    body: host.getByTestId("page-file"),
    commits: host.locator('[data-node="commits"]'),
    rail: host.getByTestId("page-rail"),
    filters: host.getByTestId("page-rail-filters"),
  };
}

/// A page's filters are in its rail, narrowing what it shows; a row click
/// picks the param the next section reads; the tab keeps both.
test("the rail's filters narrow a page, a row click shows the next section its pick, and the tab keeps both", async ({ page, daemon }) => {
  const { commits, rail, filters } = await openHistory(page, daemon);
  // The rail is the page's one place for controls: its filters as rows
  // (a label beside its value), the selection, and where it comes from.
  await expect(filters.getByTestId("page-param-range-value")).toHaveText("Last 30 days");
  await expect(filters.getByTestId("page-param-q-value")).toHaveText("Any");
  // `commit` is what a row click picks: never an input.
  await expect(rail.getByTestId("page-param-commit")).toHaveCount(0);
  await expect(rail.getByTestId("page-rail-selection")).toContainText("Click a row to select it.");
  // Copy is the page's, in its nav bar's ☰: the open run as text. About
  // this page, there too, says where it comes from.
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-page-copy")).toBeVisible();
  await page.getByTestId("menu-item-action-page-about").click();
  await expect(page.getByTestId("page-about-definition")).toHaveText("e2e-params/pages/history.yaml");
  await expect(page.getByTestId("page-about-uri")).toContainText("oxplow:oxplow/page?page=e2e_params/history");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("page-about-modal")).not.toBeVisible();

  await expect(commits).toContainText("Add the numbat");
  await expect(commits).toContainText("Add the wombat");
  await filters.getByTestId("page-param-q-value").click();
  await filters.getByTestId("page-param-q-edit").fill("numbat");
  await expect(commits).not.toContainText("Add the wombat");
  await expect(commits).toContainText("Add the numbat");

  // A pick is the next section's input (select.spec.ts checks the fill and the
  // reload); here it matters to the rail and the history.
  const row = commits.getByTestId("viz-row-0");
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(rail.getByTestId("page-rail-selection")).toContainText("Add the numbat");

  // The page's view is its tab's entry: the search and the pick are in
  // its id, so a bookmark or a reopened tab keeps them.
  const tab = shown(page, ROUTE);
  await expect(tab).toHaveAttribute("data-testid", /q=numbat/);
  await expect(tab).toHaveAttribute("data-testid", /commit=/);
  // Picking pushed an entry: Back un-selects, the filter stays, and
  // Forward finds the picked row's facts.
  await page.getByTestId("page-nav-back").filter({ visible: true }).click();
  await expect(row).toHaveAttribute("aria-selected", "false");
  await expect(rail.getByTestId("page-rail-selection")).toContainText("Click a row to select it.");
  await expect(filters.getByTestId("page-param-q-value")).toHaveText("numbat");
  await page.getByTestId("page-nav-forward").filter({ visible: true }).click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(rail.getByTestId("page-rail-selection")).toContainText("Add the numbat");
  // A reload brings the tab back with its values.
  await reload(page);
  await expect(filters.getByTestId("page-param-q-value")).toHaveText("numbat");

  // A custom range opens over the page; Escape closes it.
  await filters.getByTestId("page-param-range-value").click();
  await filters.getByTestId("page-param-range-edit").selectOption("__custom__");
  await expect(page.getByTestId("page-param-range-custom")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(shown(page).getByTestId("page-param-range-custom")).toHaveCount(0);
  await expect(filters.getByTestId("page-param-range-value")).toBeFocused();

  // Merges only: none of these is one. The section stays and says so.
  await filters.getByTestId("page-param-merges-edit").check();
  await expect(commits.getByTestId("viz-empty")).toBeVisible();
});

/// A filter is the page's view and replaces its entry; a pick is a step
/// and pushes one. Neither remounts the page.
test("a filter change keeps the page and replaces the entry; a pick pushes one Back undoes", async ({ page, daemon }) => {
  const { body, commits, filters } = await openHistory(page, daemon);
  const back = page.getByTestId("page-nav-back").filter({ visible: true });
  await expect(commits).toContainText("Add the wombat");
  await expect(back).toBeDisabled();
  // A mark on the page's own element: it must survive both changes.
  await body.evaluate((el) => el.setAttribute("data-mounted-once", "yes"));

  await filters.getByTestId("page-param-q-value").click();
  await filters.getByTestId("page-param-q-edit").fill("numbat");
  await expect(commits).not.toContainText("Add the wombat");
  await expect(body).toHaveAttribute("data-mounted-once", "yes");
  // The filter replaced the entry: nothing to go Back to.
  await expect(back).toBeDisabled();

  const row = commits.getByTestId("viz-row-0");
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(back).toBeEnabled();
  await back.click();
  await expect(row).toHaveAttribute("aria-selected", "false");
  await expect(filters.getByTestId("page-param-q-value")).toHaveText("numbat");
  await expect(body).toHaveAttribute("data-mounted-once", "yes");
  await expect(back).toBeDisabled();
});
