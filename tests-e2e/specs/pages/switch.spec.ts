import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown, reload } from "../../support/ui.js";

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
    - { id: coast, title: Coast, markdown: Dingoes of the coast. }
    - { id: desert, title: Desert, markdown: Dingoes of the desert. }
`;

test("only the switched child shows, and changing the param switches it", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-switch", { files: { "pages/dingo.yaml": spec } });
  await open(page, daemon, "oxplow:oxplow/page?page=e2e_switch/dingo");
  const host = shown(page).getByTestId("page-host");
  await expect(host.getByTestId("node-coast")).toContainText("Dingoes of the coast.");
  await expect(host.getByTestId("node-desert")).toHaveCount(0);

  // A mark on the page's own element: a section switch re-renders it, never mounts it again.
  await host.getByTestId("page-file").evaluate((el) => el.setAttribute("data-mounted-once", "yes"));

  // The rail lists the sections; picking one switches the child.
  await expect(host.getByTestId("page-section-coast")).toHaveAttribute("aria-current", "page");
  await host.getByTestId("page-section-desert").click();
  await expect(host.getByTestId("node-desert")).toContainText("Dingoes of the desert.");
  await expect(host.getByTestId("node-coast")).toHaveCount(0);
  await expect(host.getByTestId("page-section-desert")).toHaveAttribute("aria-current", "page");
  // The section is the tab's view: its id carries the param, and the page stays mounted.
  await expect(shown(page, "#area=desert")).toHaveCount(1);
  await expect(host.getByTestId("page-file")).toHaveAttribute("data-mounted-once", "yes");

  // A section pushes an entry: Back shows the first child again.
  await page.getByTestId("page-nav-back").filter({ visible: true }).click();
  await expect(host.getByTestId("node-coast")).toBeVisible();
  await expect(host.getByTestId("node-desert")).toHaveCount(0);
  await page.getByTestId("page-nav-forward").filter({ visible: true }).click();
  await expect(host.getByTestId("node-desert")).toBeVisible();

  // The choice is in the tab's address: a reload keeps it.
  await reload(page);
  await expect(host.getByTestId("node-desert")).toBeVisible();
  await expect(host.getByTestId("node-coast")).toHaveCount(0);
});
