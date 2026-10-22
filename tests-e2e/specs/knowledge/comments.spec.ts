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
  const row = inbox.getByTestId("lens-row-0");
  await expect(row).toContainText("hold hands");
  await expect(inbox.getByTestId("lens-group-heading")).toContainText("wiki/otters");

  // Picking the row reads its messages, oldest first.
  await row.click();
  await expect(inbox.getByText("Is that true?")).toBeVisible();
  await expect(inbox.getByText("Checking.")).toBeVisible();

  // Resolving takes it out of the unresolved list.
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-resolve").click();
  await expect(inbox.getByTestId("lens-row-0")).toHaveCount(0);
});
