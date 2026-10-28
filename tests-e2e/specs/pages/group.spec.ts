import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const spec = `title: Group Marsupial
category: Data
loaders:
  animals:
    sql: SELECT 'Quokka' AS name, 'Small' AS size UNION ALL SELECT 'Kangaroo', 'Large' ORDER BY 2 DESC
view:
  id: animals
  viz: table
  from: animals
  group: { by: size }
  columns: [{ key: name, label: Name }]
`;

test("rows sit under a heading per group value, and a heading folds its rows", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-group", { files: { "pages/marsupial.yaml": spec } });
  await page.goto("/");
  await openFromLauncher(page, "Group Marsupial");
  const host = page.getByTestId("page-host").filter({ visible: true });
  await expect(host.getByTestId("viz-group-heading")).toHaveCount(2);
  await expect(host).toContainText("Quokka");
  await expect(host).toContainText("Kangaroo");

  const toggle = host.getByTestId("viz-group-heading").first().locator("button[aria-expanded]");
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  // Small sorts first: its heading is the first, and only its row folds away.
  await expect(host).not.toContainText("Quokka");
  await expect(host).toContainText("Kangaroo");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(host).toContainText("Quokka");
});
