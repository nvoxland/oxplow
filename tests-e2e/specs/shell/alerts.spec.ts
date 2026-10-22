import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Alerts is the foundation's `alerts` document: a failed operation lists
// red, opens into how it ended and a link to its output, and Dismiss takes
// it off the list.
test("a failed operation is listed, opens to its output, and dismisses", async ({ page, daemon }) => {
  await run(daemon, "oxplow.ui.report_error", {
    label: "Merge bugfixes into current",
    command: "git merge bugfixes",
    message: "conflict",
    exit_code: 1,
    stderr: "CONFLICT (content): Merge conflict in a.txt",
  });
  await page.goto("/");
  await openFromLauncher(page, "Alerts");
  const part = page.locator('[data-testid="lens-part-operations"]:visible');
  await expect(part).toContainText("Merge bugfixes into current");

  // Opening the row shows how it ended and where its output is.
  await part.getByRole("button", { name: "Expand" }).click();
  await expect(part).toContainText("git merge bugfixes");
  await part.getByRole("button", { name: /part=output|oxplow:oxplow\/event/ }).click();
  await expect(page.getByText("CONFLICT (content)").first()).toBeVisible();
});
