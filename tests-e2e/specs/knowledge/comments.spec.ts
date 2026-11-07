import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// The Comments inbox is the foundation's `comments` document page: threads
// grouped by the page they are anchored to, the picked one's conversation
// under them. What it lists, its conversation and what Resolve does are the
// foundation's fixture's; the comment opened on its page is only seen here.
test("the Comments inbox opens a thread on its page", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "otters.md"), "# Otters\n\nThey hold hands.\n");
  const added = await run<{ result: { comment: { id: string } } }>(daemon, "oxplow.knowledge.add_comment", {
    stream: daemon.stream,
    target: "oxplow:oxplow/file?path=otters.md",
    quote: "hold hands",
    intent: "followup",
    body: "Is that true?",
  });
  await run(daemon, "oxplow.knowledge.reply_comment", { comment: added.result.comment.id, body: "Checking." });

  await open(page, daemon, "oxplow:oxplow/page?page=comments");
  const inbox = shown(page).getByTestId("page-file");
  // The threads are the table; the conversation under it is a list.
  const threads = inbox.getByTestId("viz-table");
  const row = threads.getByTestId("viz-row-0");
  await expect(row).toContainText("hold hands");

  // Go to location opens the page at the comment: its address carries it.
  await row.getByText("Go to location").click();
  await expect(shown(page, "oxplow:oxplow/file?path=otters.md")).toHaveAttribute(
    "data-testid",
    /#comment=cmt\d+/,
  );
  // The comment opens there, its reply box focused: Escape closes it,
  // and Back (a key the box would keep) returns to the inbox.
  const reply = page.getByTestId(`comment-reply-${added.result.comment.id}-input`).filter({ visible: true });
  await expect(reply).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(reply).toHaveCount(0);
  await page.keyboard.press("Meta+[");
  await expect(threads.getByTestId("viz-row-0")).toBeVisible();
});
