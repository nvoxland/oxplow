// The tablet shell draws the same window: its context line, the thread's
// tabs and the page, with no desktop chrome.
import { expect, test } from "../../support/fixtures.js";
import { load, open, shown } from "../../support/ui.js";

const home = "oxplow:oxplow/page?page=e2e/home";

test("with ?shell=tablet the window is the tablet shell, and a page opens in its strip", async ({ page, daemon }) => {
  await load(page, "/?shell=tablet");
  await expect(page.getByTestId("tablet-shell")).toBeVisible();
  await expect(page.getByTestId("navigator-strip")).toBeHidden();
  await expect(page.getByTestId("tablet-context")).toBeVisible();
  await open(page, daemon, home);
  await expect(page.getByTestId(`tablet-tab-${home}`)).toHaveAttribute("aria-selected", "true");
  await expect(shown(page, home)).toHaveCount(1);
});
