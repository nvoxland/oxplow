import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A folder's page is the foundation's `directory` page, the renderer of
// `oxplow/dir`: folders first, each row opens its folder or file, and the
// tab is labelled with the path.
test("a folder's URI opens its page: rows open folders and files, the tab is its path", async ({ page, daemon }) => {
  mkdirSync(join(daemon.project, "pond", "reeds"), { recursive: true });
  writeFileSync(join(daemon.project, "pond", "frog.txt"), "ribbit\n");
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");

  const uri = "oxplow:oxplow/dir?path=pond";
  await run(daemon, "oxplow.window.navigate", { uri });
  const tab = page.getByTestId(`center-tab-${uri}`);
  await expect(tab).toBeVisible();
  await expect(tab).toContainText("pond");

  const body = page.locator('[data-testid="page-host"]:visible');
  await expect(body.getByTestId("viz-row-0")).toContainText("reeds");
  await expect(body.getByTestId("viz-row-1")).toContainText("frog.txt");

  await body.getByTestId("viz-row-1").getByRole("button", { name: "frog.txt" }).click();
  await expect(page.getByTestId("center-tab-oxplow:oxplow/file?path=pond/frog.txt")).toBeVisible();
});
