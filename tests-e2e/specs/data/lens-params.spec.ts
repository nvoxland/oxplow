import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { installExample, ipc, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

/// A page made of lens files: typed params in its toolbar narrow the
/// commits, and clicking one fills the next section with its files.
test("toolbar params narrow a lens, and a row click shows the next section its pick", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=E2E", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project, stdio: "ignore" });
  for (const [file, subject] of [
    ["numbat.txt", "Add the numbat"],
    ["wombat.txt", "Add the wombat"],
  ]) {
    writeFileSync(join(daemon.project, file), `${file}\n`);
    git("add", file);
    git("commit", "-q", "-m", subject!);
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
  // The toolbar holds the toolbar params; the rail only the rail's.
  const toolbar = lens.getByTestId("lens-params-toolbar");
  await expect(toolbar.getByTestId("lens-param-range")).toBeVisible();
  await expect(page.getByTestId("lens-param-commit")).toBeVisible();
  await expect(toolbar.getByTestId("lens-param-commit")).toHaveCount(0);

  await expect(commits).toContainText("Add the numbat");
  await expect(commits).toContainText("Add the wombat");
  await toolbar.getByTestId("lens-param-q").fill("numbat");
  await expect(commits).not.toContainText("Add the wombat");
  await expect(commits).toContainText("Add the numbat");

  // Clicking the commit sets the page's `commit`: its files show below.
  const row = commits.getByTestId("lens-row-0");
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(files).toContainText("numbat.txt");

  // Merges only: none of these is one.
  await toolbar.getByTestId("lens-param-merges").check();
  // A section with nothing to show folds into the page's closing line.
  await expect(lens.getByTestId("lens-document-nothing-found")).toContainText("Commits");
});
