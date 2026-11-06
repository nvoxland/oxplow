import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Duplicate Block is the foundation's `duplicate-block` page: two `code`
// parts (a file each, read-only), their ranges marked, scrolling together.
test("a duplicate block opens its two files side by side with their ranges marked", async ({ page, daemon }) => {
  const body = Array.from({ length: 40 }, (_, i) => `line ${i + 1}`).join("\n") + "\n";
  writeFileSync(join(daemon.project, "left.txt"), body);
  writeFileSync(join(daemon.project, "right.txt"), body);
  await page.goto("/");
  await openFromLauncher(page, "Suite Home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");

  const uri =
    "oxplow:oxplow/page?left=left.txt&left_at=working&left_lines=10-14&page=duplicate-block&right=right.txt&right_at=working&right_lines=20-24";
  await run(daemon, "oxplow.window.navigate", { uri });
  const code = page.locator('[data-testid="viz-code"]:visible');
  const paths = code.getByTestId("viz-code-path");
  await expect(paths).toHaveCount(2);
  await expect(paths.nth(0)).toContainText("left.txt");
  await expect(paths.nth(1)).toContainText("right.txt");
  await expect(code.locator(".ox-code-mark").first()).toBeVisible();
});
