import { cpSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Page } from "@playwright/test";

import { installExample, ipc, run, searchable, until, type Daemon } from "../../support/daemon.js";
import { expect, test, type Workspace } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The task-views example (`examples/extensions/task-views`): swimlanes for
// the Board, a compact list for the Tasks page and a plain editor form for
// a work item, each replacing oxplow's own view with no provider of its
// own — in Chromium and WebKit.

/** Put the example into `daemon`'s project and wait until it's loaded. It
 *  isn't in every suite daemon: replacing the Board would change every
 *  other spec's. */
async function install(daemon: Daemon) {
  await installExample(daemon, "task-views");
}

/** Choose whose view shows for `target` on Settings → Integrations. */
async function choose(page: Page, target: string, who: string) {
  await openFromLauncher(page, "Settings: Integrations");
  const shows = who === "oxplow" ? "oxplow's own" : who;
  const row = page.locator("tr", { hasText: target }).filter({ hasText: shows }).first();
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-use").click();
  // The row says "In use" once the choice is stored.
  await expect(row).toContainText("In use");
}

const TARGETS = ["work_item.board", "work_item.list", "work_item.detail"] as const;

/** The state of the item titled `title`, from the models. */
async function stateOf(daemon: Daemon, title: string): Promise<string | undefined> {
  const out = await ipc<{ rows: string[][] }>(daemon, "query_sql", {
    sql: "SELECT state FROM v_work_item WHERE title = ?1",
    params: [title],
  });
  return out.rows[0]?.[0];
}

async function titled(daemon: Daemon, title: string): Promise<boolean> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT 1 FROM v_work_item WHERE title = ?1",
    params: [title],
  });
  return out.rows.length > 0;
}

/** The Board, replaced: the lanes and the drag are the example's `ui`
 *  fixture's; here the drag runs over each work list, then the rename and the
 *  empty title's refusal under its input (a `ui` key step doesn't submit a
 *  form). `scope` is the Board's scope that shows the items. */
async function swimlanes(page: Page, daemon: Workspace, scope: string) {
  await run(daemon, "oxplow.work_item.create", { title: "Card one", thread: daemon.thread });
  // A tracker's items reach oxplow's models through its collector's read.
  if (scope === "all") await run(daemon, "oxplow.provider.sync", { instance: "e2e/fake", collector: "work_items" });
  await until("the item to be in the models", 30_000, () => titled(daemon, "Card one"));
  await page.goto("/");
  await choose(page, "work_item.board", "task-views");
  await openFromLauncher(page, "Board");
  await expect(page.getByTestId("replacement-work_item.board")).toContainText("replaced by task-views");
  await page.getByTestId("page-param-show-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-show-edit").filter({ visible: true }).selectOption(scope);
  await expect(page.getByTestId("replacement-work_item.board")).toContainText("replaced by task-views");
  // Settings may still be open in another tab: its Integrations frame is a second one.
  const frame = page.frameLocator('iframe[aria-label="Swimlanes"]');
  const card = frame.locator(".card", { hasText: "Card one" });
  await expect(card).toBeVisible();

  await card.dragTo(frame.locator('.lane[data-state="done"]'));
  await expect(frame.locator('.lane[data-state="done"] .card', { hasText: "Card one" })).toBeVisible();
  expect(await stateOf(daemon, "Card one")).toBe("done");

  // The title is its own form: Enter saves, an empty one is refused under
  // the input, Escape puts it back.
  const done = frame.locator('.lane[data-state="done"] .card');
  await done.locator(".title").click();
  const input = done.locator("input.ox-input");
  await input.fill("");
  await input.press("Enter");
  await expect(done.locator(".ox-error")).toBeVisible();
  await input.press("Escape");
  await expect(done.locator(".title")).toHaveText("Card one");
  await done.locator(".title").click();
  await done.locator("input.ox-input").fill("Card renamed");
  await done.locator("input.ox-input").press("Enter");
  await expect(frame.locator(".card", { hasText: "Card renamed" })).toBeVisible();
  await until("the rename to be stored", 15_000, () => titled(daemon, "Card renamed"));
}

test("the swimlanes replace the Board: drag and rename", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await install(daemon);
  await swimlanes(page, daemon, "thread");
});

test("the swimlanes work over another work list too (e2e/fake active)", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await install(daemon);
  // The fake tracker as the active work list, set up before any page is
  // open: Settings → Integrations does the same (see fake-provider.spec).
  await run(daemon, "oxplow.provider.set_instance", { instance: "e2e/fake", enabled: true, config: { team: "core" } });
  await until("the fake to be ready", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: "SELECT 1 FROM v_provider_instance WHERE instance = 'e2e/fake' AND enabled = 1 AND state = 'ready'",
    });
    return out.rows.length === 1;
  });
  await run(daemon, "oxplow.config.set", { key: "activeProviders", value: { work_items: "fake" } });
  await until("the fake to be the active work list", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: "SELECT 1 FROM v_capability_provider WHERE capability = 'work_items' AND active = 1 AND provider = 'fake'",
    });
    return out.rows.length > 0;
  });
  // A tracker's items are on no thread.
  await swimlanes(page, daemon, "all");
});

test("the editor, the list and oxplow's own, each chosen from Settings", async ({ fresh }) => {
  // Four Settings choices twice over and three pages: about a minute on a
  // quiet machine, so the default 60s leaves it no room.
  test.setTimeout(180_000);
  const { page, daemon } = fresh;
  await install(daemon);
  await run(daemon, "oxplow.work_item.create", { title: "Item to edit", thread: daemon.thread });
  await searchable(daemon, "Item to edit", "work_item");
  await page.goto("/");
  for (const target of TARGETS) await choose(page, target, "task-views");

  // The item's page: the form in place of the description editor, the other parts still oxplow's.
  await openFromLauncher(page, "Item to edit");
  const detail = page.getByTestId("replacement-work_item.detail");
  await expect(detail).toContainText("replaced by task-views");
  // Only the fields the form names are shown; the item's ref is fixed, not editable.
  await expect(detail.getByTestId("viz-form-fields-uri")).toHaveCount(0);
  await expect(detail.getByTestId("viz-form-fields-title")).toHaveValue("Item to edit");
  await expect(page.locator('[data-testid="page-file"]:visible [data-node="state"]')).toBeVisible();
  await expect(detail.locator('[data-node="state"]')).toHaveCount(0);
  // It edits what is saved: Save waits for a change; what was typed is the
  // tab's, so a reload gives it back; Discard puts the saved title back.
  const save = detail.getByTestId("viz-form-fields-submit");
  await expect(save).toBeDisabled();
  await detail.getByTestId("viz-form-fields-title").fill("Half typed");
  await expect(save).toBeEnabled();
  await page.reload();
  await expect(detail.getByTestId("viz-form-fields-title")).toHaveValue("Half typed");
  await detail.getByTestId("viz-form-fields-discard").click();
  await expect(detail.getByTestId("viz-form-fields-title")).toHaveValue("Item to edit");
  await expect(save).toBeDisabled();
  await detail.getByTestId("viz-form-fields-title").fill("Item edited");
  await save.click();
  await until("the edit to be stored", 15_000, () => titled(daemon, "Item edited"));
  await expect(detail.getByTestId("viz-form-fields-title")).toHaveValue("Item edited");

  // The Tasks page: the compact list, its Edit… row action in a slideover.
  await openFromLauncher(page, "Tasks");
  const list = page.getByTestId("replacement-work_item.list");
  await expect(list).toContainText("replaced by task-views");
  await expect(list).toContainText("Item edited");
  // The title is a link; the row's menu is on its other cells.
  await list.getByRole("row").filter({ hasText: "Item edited" }).locator("td").nth(1).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-edit").click();
  const form = page.getByTestId("viz-action-form");
  await form.getByTestId("viz-action-form-fields-title").fill("Item edited again");
  await form.getByTestId("viz-action-form-fields-submit").click();
  await expect(form).toHaveCount(0);
  await until("the second edit to be stored", 15_000, () => titled(daemon, "Item edited again"));
  await expect(list).toContainText("Item edited again");

  // oxplow's own, for each: the replacement is gone.
  for (const target of TARGETS) await choose(page, target, "oxplow");
  await openFromLauncher(page, "Tasks");
  await expect(page.locator('[data-testid^="replacement-"]:visible')).toHaveCount(0);
  await expect(page.locator('[data-testid="page-file"]:visible [data-node="list"]').locator('[data-testid^="viz-row-"]').first()).toBeVisible();
  await openFromLauncher(page, "Board");
  await expect(page.getByTestId("viz-board")).toBeVisible();
  await expect(page.locator('[data-testid^="replacement-"]:visible')).toHaveCount(0);
  await searchable(daemon, "Item edited again", "work_item");
  await openFromLauncher(page, "Item edited again");
  await expect(page.locator('[data-testid^="replacement-"]:visible')).toHaveCount(0);
});
