import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const spec = `title: Switch Dingo
category: Data
params:
- name: area
  label: Area
  default: coast
  options:
  - { value: coast, label: Coast }
  - { value: desert, label: Desert }
view:
  switch:
    param: area
    children:
    - { id: coast, markdown: Dingoes of the coast. }
    - { id: desert, markdown: Dingoes of the desert. }
`;

test("only the switched child shows, and changing the param switches it", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-switch", { files: { "pages/dingo.yaml": spec } });
  await page.goto("/");
  await openFromLauncher(page, "Switch Dingo");
  const host = page.getByTestId("page-host").filter({ visible: true });
  await expect(host.getByTestId("node-coast")).toContainText("Dingoes of the coast.");
  await expect(host.getByTestId("node-desert")).toHaveCount(0);

  // The rail lists the sections; picking one switches the child.
  await expect(host.getByTestId("page-section-coast")).toHaveAttribute("aria-current", "page");
  await host.getByTestId("page-section-desert").click();
  await expect(host.getByTestId("node-desert")).toContainText("Dingoes of the desert.");
  await expect(host.getByTestId("node-coast")).toHaveCount(0);
  await expect(host.getByTestId("page-section-desert")).toHaveAttribute("aria-current", "page");

  // A section pushes an entry: Back shows the first child again.
  await page.getByTestId("page-nav-back").filter({ visible: true }).click();
  await expect(host.getByTestId("node-coast")).toBeVisible();
  await expect(host.getByTestId("node-desert")).toHaveCount(0);
  await page.getByTestId("page-nav-forward").filter({ visible: true }).click();
  await expect(host.getByTestId("node-desert")).toBeVisible();

  // The choice is in the tab's address: a reload keeps it.
  await page.reload();
  await expect(host.getByTestId("node-desert")).toBeVisible();
  await expect(host.getByTestId("node-coast")).toHaveCount(0);
});
