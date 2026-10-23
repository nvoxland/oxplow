import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Comments inbox is the foundation's `comments` document lens: threads
// grouped by the page they are anchored to, the picked one's conversation
// under them, and a row's actions run the comment commands.
test("the Comments inbox lists a thread, shows its conversation, and resolves it", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", { slug: "otters", title: "Otters", body: "# Otters\n\nThey hold hands.\n" });
  const added = await run<{ result: { comment: { id: string } } }>(daemon, "oxplow.knowledge.add_comment", {
    stream: daemon.stream,
    target: "oxplow:oxplow/wiki?slug=otters",
    quote: "hold hands",
    intent: "followup",
    body: "Is that true?",
  });
  await run(daemon, "oxplow.knowledge.reply_comment", { comment: added.result.comment.id, body: "Checking." });

  await page.goto("/");
  await openFromLauncher(page, "Comments");
  const inbox = page.locator('[data-testid="page-lens"]:visible');
  // The threads are the table; the conversation under it is a list.
  const threads = inbox.getByTestId("lens-table");
  const row = threads.getByTestId("lens-row-0");
  await expect(row).toContainText("hold hands");
  await expect(inbox.getByTestId("lens-group-heading")).toContainText("wiki/otters");

  // Picking the row reads its messages, oldest first.
  await row.click();
  // The conversation section is the last place each message shows.
  await expect(inbox.getByText("Is that true?").last()).toBeVisible();
  await expect(inbox.getByText("Checking.").last()).toBeVisible();

  // Go to location opens the page at the comment: its address carries it.
  await row.getByText("Go to location").click();
  await expect(page.locator('[data-testid^="center-tab-body-oxplow:oxplow/wiki?slug=otters"]:visible')).toHaveAttribute(
    "data-testid",
    /#comment=cmt\d+/,
  );
  await page.keyboard.press("Meta+[");
  await expect(threads.getByTestId("lens-row-0")).toBeVisible();

  // Resolving takes it out of the unresolved list.
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-resolve").click();
  await expect(threads.getByTestId("lens-row-0")).toHaveCount(0);
});
