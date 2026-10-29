import { rmSync } from "node:fs";
import { join } from "node:path";
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

test("show_page opens a temp page (a file on oxplow:temp) whose live run survives reload", async ({ page, daemon }) => {
  const uri = await show(page, daemon, source("Shown Quoll Count", "Shown Quoll "));
  expect(uri).toContain("oxplow:oxplow/page?page=temp/");
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

test("Save as Page writes the view's file as a deployed page and opens it", async ({ page, daemon }) => {
  await show(page, daemon, source("Saved Wombat Count", "Saved Wombat "));
  await action(page, "view-save-page");
  const form = page.getByTestId("save-view-form").filter({ visible: true });
  await form.getByTestId("save-view-title").fill("Saved Wombat View");
  await form.getByTestId("save-view-title").press("Enter");
  await expect(page.getByTestId("center-tab-oxplow:oxplow/page?page=my-pages/saved-wombat-view")).toBeVisible();
  await expect(page.getByTestId("page-file").filter({ visible: true }).locator('[data-node="count"]')).toContainText("0");
});

test("Show Definition on a view opens Definition of its temp file", async ({ page, daemon }) => {
  await show(page, daemon, source("Defined Potoroo Count", "Defined Potoroo "));
  await action(page, "page-definition");
  const file = page.getByTestId("page-file").filter({ visible: true });
  await expect(file.locator("h1")).toHaveText(/^Definition: v[0-9a-f]{16}\.yaml$/);
  await expect(file.getByTestId("node-about")).toContainText("Defined Potoroo Count");
  await expect(file.getByTestId("node-about")).toContainText("temp/v");
});

test("a view whose file is gone says so, and the tab stays", async ({ page, daemon }) => {
  const uri = await show(page, daemon, source("Gone Numbat Count", "Gone Numbat "));
  const id = uri.slice(uri.lastIndexOf("/") + 1);
  rmSync(join(daemon.project, ".oxplow", "runtime", "temp", `${id}.yaml`));
  await page.reload();
  await expect(page.getByTestId(`center-tab-${uri}`)).toBeVisible();
  await expect(page.getByTestId("view-gone").filter({ visible: true })).toContainText("This view is gone");
});
