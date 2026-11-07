import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// What the Uncommitted page lists and what its commit form commits is the
// foundation's fixtures'. A throwaway oxplow does not watch files, so only
// the app shows the page drawn again once the commit is made.
test("the uncommitted page drops the files a commit took", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "wallaby.txt"), "hop\n");
  await open(page, daemon, "oxplow:oxplow/page?page=uncommitted-changes");
  const body = shown(page).getByTestId("page-file");
  const files = body.locator('[data-node="files"]');
  await expect(files.getByRole("button", { name: "wallaby.txt" })).toBeVisible();

  await body.getByTestId("viz-form-fields-message").fill("Add the wallaby file");
  await body.getByTestId("viz-form-fields-submit").click();

  // (The project has other uncommitted files, the test extension's among
  // them, so the list doesn't go empty.)
  await expect(files.getByRole("button", { name: "wallaby.txt" })).toHaveCount(0);
});
