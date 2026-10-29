import type { Page } from "@playwright/test";
import { expect, test } from "../../support/fixtures.js";
import { run } from "../../support/daemon.js";
import { openFromLauncher } from "../../support/ui.js";

type Daemon = Parameters<typeof run>[0];
const source = (title: string, prefix: string) => `# Keep the source comment
title: ${title}
loaders:
  count:
    sql: SELECT count(*) AS n FROM v_work_item WHERE title LIKE '${prefix}%'
view: { id: count, viz: number, from: count }
`;

async function show(page: Page, daemon: Daemon, spec: string) {
  await page.goto("/");
  // An open page and settled rail mean the window can answer a show call.
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const shown = await run<{ result: { uri: string; opened: boolean; why?: string } }>(daemon, "oxplow.page.show", { spec });
  expect(shown.result.opened, shown.result.why).toBe(true);
  const tab = page.getByTestId(`center-tab-${shown.result.uri}`);
  await expect(tab).toBeVisible();
  await tab.click();
  return shown.result.uri;
}

async function action(page: Page, id: string) {
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId(`menu-item-action-${id}`).click();
}

test("show_page opens an ephemeral view whose exact source and live run survive reload", async ({ page, daemon }) => {
  const uri = await show(page, daemon, source("Shown Quoll Count", "Shown Quoll "));
  const body = page.getByTestId("page-file").filter({ visible: true });
  await expect(body).toContainText("Shown Quoll Count");
  await expect(body.locator('[data-node="count"]')).toContainText("0");
  await page.reload();
  await expect(body).toContainText("Shown Quoll Count");
  await run(daemon, "oxplow.work_item.create", { title: "Shown Quoll evidence", thread: daemon.thread });
  await expect(body.locator('[data-node="count"]')).toContainText("1");
  await expect(page.getByTestId(`center-tab-${uri}`)).toBeVisible();
});

test("Save to wiki keeps a live oxplow-page block that renders again after reload", async ({ page, daemon }) => {
  await show(page, daemon, source("Saved Bilby Count", "Saved Bilby "));
  await action(page, "view-save-wiki");
  const form = page.getByTestId("save-view-form").filter({ visible: true });
  await form.getByTestId("save-view-title").fill("Saved Bilby View");
  await form.getByTestId("save-view-title").press("Enter");
  const inline = page.getByTestId("inline-page").filter({ visible: true });
  await expect(inline.locator('[data-node="count"]')).toContainText("0");
  await page.reload();
  await expect(inline.locator('[data-node="count"]')).toContainText("0");
  await run(daemon, "oxplow.work_item.create", { title: "Saved Bilby evidence", thread: daemon.thread });
  await expect(inline.locator('[data-node="count"]')).toContainText("1");
});

test("an inline view has no Show definition: it has no file to describe", async ({ page, daemon }) => {
  await show(page, daemon, source("Defined Potoroo Count", "Defined Potoroo "));
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-view-save-page")).toBeVisible();
  await expect(page.getByTestId("menu-item-action-page-definition")).toHaveCount(0);
});
