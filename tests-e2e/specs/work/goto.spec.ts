import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher, shown } from "../../support/ui.js";

// A bookmark is of the page's whole address, view included. The same page at
// another view shows the star as "bookmarked at other values", and its popover
// moves that bookmark to the view on screen (one undoable command).
test("the star offers to move a bookmark made at another filter value", async ({ page, daemon }) => {
  const bookmarked = async () =>
    (await ipc<{ rows: string[][] }>(daemon, "query_sql", { sql: "SELECT uri FROM v_bookmark" })).rows.map((r) => String(r[0]));
  await page.goto("/");
  await openFromLauncher(page, "Local History");
  const star = page.getByTestId("page-nav-bookmark").filter({ visible: true });
  await star.click();
  await page.getByTestId("page-nav-bookmark-scope-thread").filter({ visible: true }).click();
  await expect(star).toHaveText("★");
  // The star's popover closes with a second click on the star.
  await star.click();
  await expect(page.getByTestId("page-nav-bookmark-popover").filter({ visible: true })).not.toBeVisible();
  await expect.poll(bookmarked).toEqual([expect.stringContaining("local-history")]);
  const before = (await bookmarked())[0];
  expect(before).not.toContain("mode=by-commit");

  // Another value of the page's filter: the star is off, but says why.
  await shown(page).getByTestId("page-param-mode-value").click();
  await shown(page).getByTestId("page-param-mode-edit").selectOption("by-commit");
  await expect(star).toHaveText("☆");
  await expect(star).toHaveAttribute("title", "Bookmarked at other values");
  // The filter is in the tab's address.
  await expect(shown(page, "#mode=by-commit")).toHaveCount(1);

  await star.click();
  await page.getByTestId("page-nav-bookmark-update").filter({ visible: true }).click();
  await expect(star).toHaveText("★");
  await expect.poll(bookmarked).toEqual([expect.stringContaining("mode=by-commit")]);

  // The move was one command, so one Undo puts the bookmark back at its old view.
  const audit = await ipc<{ rows: number[][] }>(daemon, "query_sql", {
    sql: "SELECT id FROM v_command_audit WHERE command = 'oxplow.command.sequence' AND outcome = 'ok' ORDER BY id DESC LIMIT 1",
  });
  expect(audit.rows).toHaveLength(1);
  await ipc(daemon, "undo_command", { auditId: audit.rows[0]![0], confirmed: true });
  await expect.poll(bookmarked).toEqual([before]);

  // The shell recorded the visit when the page opened: Go To lists it, beside the bookmark.
  await openFromLauncher(page, "Go To");
  await expect(page.locator('[data-section="bookmarks"]:visible')).toContainText("Local History");
  await expect(page.locator('[data-section="recent"]:visible')).not.toContainText("No visits yet");
});
