import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import type { Page } from "@playwright/test";

import { installExample, ipc, until, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

/// Two commits for the commit-history example, indexed, and the example
/// deployed: a page whose filters narrow its commits and whose rows pick
/// the `commit` the next section reads.
async function twoCommits(daemon: Daemon & { project: string }) {
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=E2E", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project, stdio: "ignore" });
  for (const [file, subject] of [
    ["numbat.txt", "Add the numbat"],
    ["wombat.txt", "Add the wombat"],
  ] as const) {
    writeFileSync(join(daemon.project, file), `${file}\n`);
    git("add", file);
    git("commit", "-q", "-m", subject);
  }
  await until("the commits are indexed", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_commit WHERE subject LIKE 'Add the %'" });
    return out.rows[0]?.[0] === 2;
  });
  await installExample(daemon, "commit-history");
}

const ROUTE = "oxplow:oxplow/page?page=commit-history/history";

async function openHistory(page: Page) {
  await searchLauncher(page, "Commit history");
  await page.getByTestId(`launcher-page-${ROUTE}`).click();
  const host = page.getByTestId("page-host").filter({ visible: true });
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
test("the rail's filters narrow a page, a row click shows the next section its pick, and the tab keeps both", async ({ fresh }) => {
  const { page, daemon } = fresh;
  // The window is up while the commits land, as a person's is.
  await page.goto("/");
  await twoCommits(daemon);
  const { body, commits, rail, filters } = await openHistory(page);
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
  await expect(page.getByTestId("page-about-definition")).toHaveText("commit-history/pages/history.yaml");
  await expect(page.getByTestId("page-about-uri")).toContainText("oxplow:oxplow/page?page=commit-history/history");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("page-about-modal")).toHaveCount(0);

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
  const tab = page.locator(`[data-testid^="center-tab-body-${ROUTE}"]:visible`);
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
  await page.reload();
  await expect(filters.getByTestId("page-param-q-value")).toHaveText("numbat");

  // A custom range opens over the page; Escape closes it.
  await filters.getByTestId("page-param-range-value").click();
  await filters.getByTestId("page-param-range-edit").selectOption("__custom__");
  await expect(page.getByTestId("page-param-range-custom")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("page-param-range-custom")).toHaveCount(0);
  await expect(filters.getByTestId("page-param-range-value")).toBeFocused();

  // Merges only: none of these is one. The section stays and says so.
  await filters.getByTestId("page-param-merges-edit").check();
  await expect(commits.getByTestId("viz-empty")).toBeVisible();
});

/// A filter is the page's view and replaces its entry; a pick is a step
/// and pushes one. Neither remounts the page.
test("a filter change keeps the page and replaces the entry; a pick pushes one Back undoes", async ({ fresh }) => {
  const { page, daemon } = fresh;
  // The window is up while the commits land, as a person's is.
  await page.goto("/");
  await twoCommits(daemon);
  const { body, commits, filters } = await openHistory(page);
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
