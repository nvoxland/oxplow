import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A work item's page is the foundation's `work-item` document: a part each
// for its state, title and fields, description, where it is, links,
// comments and efforts.
async function openItem(page: import("@playwright/test").Page, title: string) {
  await page.goto("/");
  await openFromLauncher(page, "Board");
  await page.getByTestId("lens-board").getByText(title).click();
  await expect(page.locator('[data-node="info"]')).toBeVisible();
}

test("the state and the title are saved from their forms", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Item to move", thread: daemon.thread });
  await openItem(page, "Item to move");

  const state = page.locator('[data-node="state"]');
  await state.getByTestId("lens-form-fields-to").selectOption("in_progress");
  await state.getByTestId("lens-form-fields-submit").click();

  const fields = page.locator('[data-node="fields"]');
  await fields.getByTestId("lens-form-fields-title").fill("Item moved and renamed");
  await fields.getByTestId("lens-form-fields-submit").click();

  await openFromLauncher(page, "Board");
  await expect(page.getByTestId("lens-board-column-in_progress")).toContainText("Item moved and renamed");
});

test("Comment… and Link… add a comment and a link, and Unlink removes the link", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Item with notes", thread: daemon.thread });
  await run(daemon, "oxplow.work_item.create", { title: "Item to link to", thread: daemon.thread });
  const other = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Third item",
    thread: daemon.thread,
  });
  await openItem(page, "Item with notes");

  await page.locator('[data-testid="page-file"]:visible [data-node="comments"]').getByTestId("lens-action-comment").click();
  const comment = page.getByTestId("lens-action-form");
  await comment.getByTestId("lens-action-form-fields-body").fill("A note for later.");
  await comment.getByTestId("lens-action-form-fields-submit").click();
  await expect(page.locator('[data-testid="page-file"]:visible [data-node="comments"]')).toContainText("A note for later.");

  await page.locator('[data-node="links"]').getByTestId("lens-action-link").click();
  const link = page.getByTestId("lens-action-form");
  await link.getByTestId("lens-action-form-fields-target").fill(other.result.uri);
  await link.getByTestId("lens-action-form-fields-submit").click();
  const links = page.locator('[data-node="links"]');
  await expect(links).toContainText("Third item");

  await links.getByRole("row").filter({ hasText: "Third item" }).locator("td").first().click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-unlink").click();
  await expect(links).not.toContainText("Third item");
});

test("About offers Send to backlog for an item on the thread, and Delete asks first", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Item to send", thread: daemon.thread });
  await openItem(page, "Item to send");

  const about = page.locator('[data-node="info"]').getByTestId("lens-detail");
  await about.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-send").click();
  await expect(about).toContainText("Backlog");

  await about.click({ button: "right" });
  await expect(page.getByTestId("menu-item-lens-action-send")).toHaveCount(0);
  await page.getByTestId("menu-item-lens-action-delete").click();
  await expect(page.getByTestId("lens-action-confirm-run")).toBeVisible();
  // Confirmed, the item is gone and its page closes with it.
  await page.getByTestId("lens-action-confirm-run").click();
  await expect(page.locator('[data-node="info"]')).toHaveCount(0);
});
