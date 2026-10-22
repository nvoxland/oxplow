import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Uncommitted Changes is the foundation's `uncommitted` document: the
// working tree's files as a tree, and a commit form over `oxplow.vcs.commit`.
test("the uncommitted page lists the working tree and commits it", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "wallaby.txt"), "hop\n");
  await page.goto("/");
  await openFromLauncher(page, "Uncommitted");
  const body = page.locator('[data-testid="page-lens"]:visible');
  // The Files section's: the change analysis other extensions add lists
  // the file too, once it lands.
  const files = body.getByTestId("lens-part-files");
  await expect(files.getByRole("button", { name: "wallaby.txt" })).toBeVisible();

  await body.getByTestId("lens-form-fields-message").fill("Add the wallaby file");
  await body.getByTestId("lens-form-fields-submit").click();

  // The commit is in git, and the file leaves the page's list. (The
  // project has other uncommitted files, the test extension's among them,
  // so the list doesn't go empty.)
  await expect
    .poll(() => execFileSync("git", ["log", "--format=%s"], { cwd: daemon.project }).toString())
    .toContain("Add the wallaby file");
  await expect(files.getByRole("button", { name: "wallaby.txt" })).toHaveCount(0);
  await openFromLauncher(page, "Git History");
  await expect(
    page.frameLocator('[data-testid="custom-component-frame"]').locator(".row", { hasText: "Add the wallaby file" }),
  ).toBeVisible();
});
