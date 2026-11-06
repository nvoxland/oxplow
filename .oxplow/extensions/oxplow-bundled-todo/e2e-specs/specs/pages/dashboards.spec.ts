import type { Page } from "@playwright/test";

import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

type Daemon = Parameters<typeof ipc>[0];
interface DashboardPage {
  title: string;
  view: { kind: string; children: Array<{ id: string; kind: string; page: string }> };
}

const activeHost = (page: Page) => page.getByTestId("page-host").filter({ visible: true });
const activeGrid = (page: Page) => page.getByTestId("dashboard-grid").filter({ visible: true });

function dashboard(daemon: Daemon, id: string) {
  return ipc<DashboardPage>(daemon, "get_page", { id });
}

test("Dashboards creates an empty page from its title form and opens it", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Dashboards");
  await expect(activeHost(page)).toContainText("Dashboards");
  await page.getByTestId("action-page-action-new").filter({ visible: true }).click();
  const title = page.getByTestId("viz-action-form-fields-title").filter({ visible: true });
  await title.fill("Quokka release dashboard");
  await title.press("Enter");
  await expect(page.getByTestId("dashboard-title-edit").filter({ visible: true })).toHaveText("Quokka release dashboard");
  await expect(activeGrid(page).locator(':scope > [data-testid^="dashboard-tile-"]')).toHaveCount(0);
  const id = "my-pages/quokka-release-dashboard";
  await expect.poll(async () => (await dashboard(daemon, id)).view.children).toEqual([]);

  // It is a normal page in the Dashboards index, including after reload.
  await page.reload();
  await openFromLauncher(page, "Dashboards");
  // Other specs may have dashboards too, so find the linked row by its
  // title rather than depending on its ordinal position in the index.
  const linked = activeHost(page).locator('[data-testid^="viz-row-"]').filter({ hasText: "Quokka release dashboard" });
  await expect(linked).toHaveCount(1);
  await linked.getByText("Quokka release dashboard", { exact: true }).click();
  await expect(page.getByTestId("dashboard-title-edit").filter({ visible: true })).toHaveText("Quokka release dashboard");
});
