import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { navigate, shown } from "../../support/ui.js";

// What each file is offered, read-only or not, and the "binary file" wording's data, is
// the foundation's fixtures'; one smoke that the Text view draws read-only here.

type Page = import("@playwright/test").Page;

test("a file on the running extensions opens read-only in the Text view", async ({ page, daemon }) => {
  const uri = "oxplow:oxplow/file?path=oxplow-foundation/extension.yaml&filesystem=oxplow:extensions";
  await navigate(page, daemon, uri);
  const code = shown(page).getByTestId("viz-code");
  await expect(code).toContainText("oxplow-foundation");
  await expect(shown(page).getByTestId("viz-editor")).toHaveCount(0);
  // No Save: the tab has no dirty mark and File → Save has nothing to write.
  await expect(page.getByTestId(`center-tab-dirty-${uri}`)).not.toBeVisible();
  await expect(shown(page).getByTestId("viz-code-path")).toHaveText("oxplow-foundation/extension.yaml");
  await expect(shown(page).getByTestId("viz-code-file")).toContainText("@oxplow:extensions");
  // A page that fills, with no filter, selection or rail node, has no
  // Details card.
  await expect(shown(page).getByTestId("page-rail")).toHaveCount(0);
});
