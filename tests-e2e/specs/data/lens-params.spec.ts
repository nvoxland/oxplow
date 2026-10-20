import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { installExample, ipc, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

/// A page made of lens files: typed params in its toolbar narrow the
/// commits, and clicking one fills the next section with its files.
test("the rail's params narrow a lens, and a row click shows the next section its pick", async ({ fresh }) => {
  const { page, daemon } = fresh;
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
  await until("the commits are indexed", 15_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_commit WHERE subject LIKE 'Add the %'" });
    return out.rows[0]?.[0] === 2;
  });
  await installExample(daemon, "commit-history");

  await page.goto("/");
  await searchLauncher(page, "Commit history");
  await page.getByTestId("launcher-page-lens:commit-history/history").click();
  const lens = page.getByTestId("page-lens");
  const commits = lens.getByTestId("lens-part-commits");
  const files = lens.getByTestId("lens-part-files");
  // The rail is the page's one place for controls: its filters as rows
  // (a label beside its value), the selection, and where it comes from.
  const rail = lens.getByTestId("page-rail");
  const filters = rail.getByTestId("lens-rail-filters");
  await expect(filters.getByTestId("lens-param-range-value")).toHaveText("Last 30 days");
  await expect(filters.getByTestId("lens-param-q-value")).toHaveText("Any");
  // `commit` is what a row click picks: never an input.
  await expect(rail.getByTestId("lens-param-commit")).toHaveCount(0);
  await expect(rail.getByTestId("lens-rail-selection")).toContainText("Click a row to select it.");
  await expect(rail.getByTestId("lens-about-source")).toHaveText(".oxplow/extensions/commit-history/lenses/history.yaml");
  // Copy and Add to Agent Context are the page's, in its nav bar.
  await expect(page.getByTestId("action-lens-copy")).toBeVisible();
  await expect(lens.getByTestId("lens-actions")).toHaveCount(0);

  await expect(commits).toContainText("Add the numbat");
  await expect(commits).toContainText("Add the wombat");
  await filters.getByTestId("lens-param-q-value").click();
  await filters.getByTestId("lens-param-q-edit").fill("numbat");
  await expect(commits).not.toContainText("Add the wombat");
  await expect(commits).toContainText("Add the numbat");

  // Clicking the commit sets the page's `commit`: its files show below.
  const row = commits.getByTestId("lens-row-0");
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(files).toContainText("numbat.txt");
  await expect(rail.getByTestId("lens-rail-selection")).toContainText("Add the numbat");

  // A custom range opens over the page; Escape closes it.
  await filters.getByTestId("lens-param-range-value").click();
  await filters.getByTestId("lens-param-range-edit").selectOption("__custom__");
  await expect(page.getByTestId("lens-param-range-custom")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("lens-param-range-custom")).toHaveCount(0);
  await expect(filters.getByTestId("lens-param-range-value")).toBeFocused();

  // Merges only: none of these is one.
  await filters.getByTestId("lens-param-merges-edit").check();
  // A section with nothing to show folds into the page's closing line.
  await expect(lens.getByTestId("lens-document-nothing-found")).toContainText("Commits");
});
