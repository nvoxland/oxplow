import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

test("moving a card transitions its item, and Undo moves it back", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Card to move", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Board");
  const todo = page.getByTestId("board-column-todo");
  const inProgress = page.getByTestId("board-column-in_progress");
  // Anywhere on the card, its title link too: the card's menu.
  await todo.getByTestId("board-card").filter({ hasText: "Card to move" }).click({ button: "right" });
  await page.getByTestId("menu-item-board-move-in_progress").click();
  await expect(inProgress).toContainText("Card to move");
  await expect(todo).not.toContainText("Card to move");
  await page.getByTestId("undo-toast-undo").click();
  await expect(todo).toContainText("Card to move");
  await expect(inProgress).not.toContainText("Card to move");
});

test("deleting an item asks first, then it's gone from the board", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Card to delete", thread: daemon.thread });
  await run(daemon, "oxplow.work_item.create", { title: "Card to keep", thread: daemon.thread });
  await page.goto("/");
  await openFromLauncher(page, "Board");
  await page.getByTestId("board-card").filter({ hasText: "Card to delete" }).getByText("Card to delete").click();
  await page.getByTestId("task-rail-delete-trigger").click();
  // Asked, and nothing is gone until the person confirms.
  await expect(page.getByTestId("task-rail-delete-confirm")).toBeVisible();
  await page.getByTestId("task-rail-delete-confirm").click();
  await openFromLauncher(page, "Board");
  // The board has loaded its cards: what's absent is gone, not unloaded.
  await expect(page.getByTestId("work-board")).toContainText("Card to keep");
  await expect(page.getByTestId("work-board")).not.toContainText("Card to delete");
});
