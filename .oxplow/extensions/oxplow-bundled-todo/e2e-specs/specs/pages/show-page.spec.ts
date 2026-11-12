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
    sql: SELECT count(*) AS n FROM v_oxplow_task WHERE title LIKE '${prefix}%'
view: { id: count, viz: number, from: count }
`;

async function show(page: Page, daemon: Daemon, spec: string) {
  await page.goto("/");
  // An open page and settled rail mean the window can answer a show call.
  await openFromLauncher(page, "Git Status");
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

test("Save as knowledge page keeps a live oxplow-page block that renders again after reload", async ({ page, daemon }) => {
  await show(page, daemon, source("Saved Bilby Count", "Saved Bilby "));
  await action(page, "view-save-knowledge");
  const form = page.getByTestId("save-view-form").filter({ visible: true });
  await form.getByTestId("save-view-title").fill("Saved Bilby View");
  await form.getByTestId("save-view-title").press("Enter");
  const inline = page.getByTestId("inline-page").filter({ visible: true });
  await expect(inline.locator('[data-node="count"]')).toContainText("0");
  await page.reload();
  await expect(inline.locator('[data-node="count"]')).toContainText("0");
  await run(daemon, "oxtask.task.create", { title: "Saved Bilby evidence" });
  await expect(inline.locator('[data-node="count"]')).toContainText("1");
});
