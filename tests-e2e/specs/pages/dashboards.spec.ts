import type { Page } from "@playwright/test";

import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown, reload } from "../../support/ui.js";

type Daemon = Parameters<typeof ipc>[0];
interface DashboardPage {
  title: string;
  view: { kind: string; children: Array<{ id: string; kind: string; page: string }> };
}

const activeHost = (page: Page) => shown(page).getByTestId("page-host");
const activeGrid = (page: Page) => shown(page).getByTestId("dashboard-grid");

function dashboard(daemon: Daemon, id: string) {
  return ipc<DashboardPage>(daemon, "get_page", { id });
}

async function pin(page: Page, daemon: Daemon, slug: string, title: string, dashboardId: string) {
  await open(page, daemon, `oxplow:oxplow/page?page=e2e-dashboard-pages/${slug}`);
  await expect(activeHost(page)).toContainText(title);
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-page-pin").click();
  await page.getByTestId(`page-pin-to-${dashboardId}`).click();
  await expect(activeGrid(page)).toBeVisible();
}

test("pinning, reordering, removing and renaming edit the dashboard page and survive reload", async ({ page, daemon }) => {
  const extension = "e2e-dashboard-pages";
  const id = `${extension}/dunnart-board`;
  for (const [slug, title, text] of [
    ["tests", "Dunnart test summary", "Dunnart test evidence"],
    ["coverage", "Dunnart coverage summary", "Dunnart coverage evidence"],
  ]) {
    await run(daemon, "oxplow.extension.write_page", {
      extension, page: slug,
      spec: `# The deployed ${slug} page\ntitle: ${title}\ncategory: Data\nview: { markdown: ${text} }\n`,
      deploy: true,
    });
  }
  await run(daemon, "oxplow.extension.write_page", {
    extension, page: "dunnart-board",
    spec: "# Preserve this dashboard explanation\ntitle: Dunnart board\ncategory: Dashboards\nview: { grid: [] }\n",
    deploy: true,
  });
  await pin(page, daemon, "tests", "Dunnart test summary", id);
  await expect(activeGrid(page)).toContainText("Dunnart test evidence");
  await pin(page, daemon, "coverage", "Dunnart coverage summary", id);
  await expect(activeGrid(page)).toContainText("Dunnart coverage evidence");
  await expect.poll(async () => (await dashboard(daemon, id)).view.children.map((tile) => tile.page))
    .toEqual([`${extension}/tests`, `${extension}/coverage`]);
  const pinned = (await dashboard(daemon, id)).view.children;
  const first = pinned[0]!;
  const second = pinned[1]!;
  const tile = (nodeId: string) => activeGrid(page).getByTestId(`dashboard-tile-${nodeId}`);

  await expect(tile(first.id)).toHaveAttribute("draggable", "true");
  await tile(first.id).dragTo(tile(second.id));
  await expect.poll(async () => (await dashboard(daemon, id)).view.children.map((node) => node.id))
    .toEqual([second.id, first.id]);
  await expect(activeGrid(page).locator(':scope > [data-testid^="dashboard-tile-"]')).toHaveText([
    /Dunnart coverage evidence/, /Dunnart test evidence/,
  ]);

  // Removal belongs to the tile's right-click menu, and asks inline; it is
  // offered once the reorder's save has finished (the grid isn't busy).
  await expect(activeGrid(page)).not.toHaveAttribute("aria-busy", "true");
  await tile(first.id).click({ button: "right" });
  await page.getByTestId("menu-item-remove").click();
  await tile(first.id).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(tile(first.id)).toBeVisible();
  expect((await dashboard(daemon, id)).view.children).toHaveLength(2);
  await tile(first.id).click({ button: "right" });
  await page.getByTestId("menu-item-remove").click();
  await tile(first.id).getByRole("button", { name: "Confirm", exact: true }).click();
  await expect.poll(async () => (await dashboard(daemon, id)).view.children.map((node) => node.id))
    .toEqual([second.id]);
  await expect(tile(first.id)).toHaveCount(0);

  await shown(page).getByTestId("dashboard-title-edit").click();
  const renamed = shown(page).getByTestId("dashboard-title-input");
  await renamed.fill("Dunnart release overview");
  await renamed.press("Enter");
  await expect.poll(async () => (await dashboard(daemon, id)).title).toBe("Dunnart release overview");
  const saved = await ipc<{ yaml: string }>(daemon, "page_spec", { page: id });
  expect(saved.yaml).toContain("# Preserve this dashboard explanation");

  await reload(page);
  await expect(shown(page).getByTestId("dashboard-title-edit")).toHaveText("Dunnart release overview");
  await expect(activeGrid(page)).toContainText("Dunnart coverage evidence");
  await expect(activeGrid(page)).not.toContainText("Dunnart test evidence");
  await expect(activeGrid(page).locator(':scope > [data-testid^="dashboard-tile-"]')).toHaveCount(1);
});
