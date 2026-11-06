import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// What each file is offered, read-only or not, and the "binary file" wording's data, is
// the foundation's fixtures'; one smoke that the Text view draws read-only here.

type Page = import("@playwright/test").Page;

async function ready(page: Page) {
  await page.goto("/");
  await openFromLauncher(page, "Suite Home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
}

const navigate = (daemon: Parameters<typeof run>[0], uri: string) => run(daemon, "oxplow.window.navigate", { uri });

test("a file on the running extensions opens read-only in the Text view", async ({ page, daemon }) => {
  await ready(page);
  const uri = "oxplow:oxplow/file?path=oxplow-foundation/extension.yaml&filesystem=oxplow:extensions";
  await navigate(daemon, uri);
  const code = page.getByTestId("viz-code").filter({ visible: true });
  await expect(code).toContainText("oxplow-foundation");
  await expect(page.getByTestId("viz-editor")).toHaveCount(0);
  // No Save: the tab has no dirty mark and File → Save has nothing to write.
  await expect(page.getByTestId(`center-tab-dirty-${uri}`)).toHaveCount(0);
  await expect(page.getByTestId("viz-code-path").filter({ visible: true })).toHaveText("oxplow-foundation/extension.yaml");
  await expect(page.getByTestId("viz-code-file").filter({ visible: true })).toContainText("@oxplow:extensions");
  // A page that fills, with no filter, selection or rail node, has no
  // Details card.
  await expect(page.getByTestId("page-rail").filter({ visible: true })).toHaveCount(0);
});
