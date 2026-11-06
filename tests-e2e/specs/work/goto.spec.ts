import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Go To is the foundation's page: the starred page is a bookmark row
// there, and Recently visited lists pages.
test("the Go To page lists a starred page and the pages visited", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Local History");
  await page.locator('[data-testid="page-nav-bookmark"]:visible').click();
  await page.locator('[data-testid="page-nav-bookmark-scope-thread"]:visible').click();
  await openFromLauncher(page, "Go To");
  const goTo = page.locator('[data-section="bookmarks"]:visible');
  await expect(goTo).toContainText("Local History");
  await expect(page.locator('[data-section="recent"]:visible')).not.toContainText("No visits yet");
});

// Local History is the foundation's document of two pages: its snapshots
// recent, or by git commit (the `mode` param, in the address).
test("the Local History page draws its snapshots and keeps its grouping in the address", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Local History");
  const body = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid$="page=local-history"]:visible');
  await expect(body.getByTestId("page-nav-title")).toHaveText("Local History");
  await page.getByTestId("page-param-mode-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-mode-edit").filter({ visible: true }).selectOption("by-commit");
  await expect(page.locator('[data-testid="center-tab-body-oxplow:oxplow/page?page=local-history#mode=by-commit"]:visible')).toHaveCount(1);
});

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
  await expect(page.getByTestId("page-nav-bookmark-popover").filter({ visible: true })).toHaveCount(0);
  await expect.poll(bookmarked).toEqual([expect.stringContaining("local-history")]);
  const before = (await bookmarked())[0];
  expect(before).not.toContain("mode=by-commit");

  // Another value of the page's filter: the star is off, but says why.
  await page.getByTestId("page-param-mode-value").filter({ visible: true }).click();
  await page.getByTestId("page-param-mode-edit").filter({ visible: true }).selectOption("by-commit");
  await expect(star).toHaveText("☆");
  await expect(star).toHaveAttribute("title", "Bookmarked at other values");

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
});
