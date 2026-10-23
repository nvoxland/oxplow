import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Board is the foundation's `board` document: a `viz: board` of the
// work items, a column per state, a card dropped on a column transitioning
// it through the work list.
test("dragging a card to a column transitions its item, and Undo moves it back", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Card to move", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Board");
  const todo = page.getByTestId("lens-board-column-todo");
  const inProgress = page.getByTestId("lens-board-column-in_progress");
  await expect(todo).toContainText("Card to move");
  // A real drag: the card onto the In Progress column.
  await todo.getByRole("listitem").filter({ hasText: "Card to move" }).dragTo(inProgress);
  await expect(inProgress).toContainText("Card to move");
  await expect(todo).not.toContainText("Card to move");
  await page.getByTestId("undo-toast-undo").click();
  await expect(todo).toContainText("Card to move");
  await expect(inProgress).not.toContainText("Card to move");
});

test("a card's menu moves it too", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Card in the menu", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Board");
  const todo = page.getByTestId("lens-board-column-todo");
  await todo.getByRole("listitem").filter({ hasText: "Card in the menu" }).click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-to-done").click();
  await expect(page.getByTestId("lens-board-column-done")).toContainText("Card in the menu");
  await expect(todo).not.toContainText("Card in the menu");
});

test("deleting an item asks first, then it's gone from the board", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Card to delete", thread: daemon.thread });
  await run(daemon, "oxplow.work_item.create", { title: "Card to keep", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Board");
  await page.getByTestId("lens-board").getByText("Card to delete").click();
  await page.getByTestId("lens-part-info").getByTestId("lens-detail").click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-delete").click();
  // Asked, and nothing is gone until the person confirms.
  await expect(page.getByTestId("lens-action-confirm-run")).toBeVisible();
  await page.getByTestId("lens-action-confirm-run").click();
  await openFromLauncher(page, "Board");
  // The board has loaded its cards: what's absent is gone, not unloaded.
  await expect(page.getByTestId("lens-board")).toContainText("Card to keep");
  await expect(page.getByTestId("lens-board")).not.toContainText("Card to delete");
});
