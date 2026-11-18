import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// A page's ☰ → Add to Context opens the Context panel's add card, then puts
// the page in the selected thread's basket: the dock's Context icon counts
// it, the panel lists it with the note, and the × takes it out again.
test("a page added from its ☰ is counted in the dock and removed from its panel", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await page.locator('[data-testid="page-nav-menu"]:visible').click();
  await page.getByTestId("menu-item-page-add-to-context").click();
  await expect(page.getByTestId("context-panel")).toBeVisible();
  await page.getByTestId("context-note-field").fill("Check the greeting.");
  await page.getByTestId("context-note-add").click();
  await expect(page.getByTestId("context-add-card")).not.toBeVisible();

  const badge = page.getByTestId("dock-context-badge");
  await expect(badge).toHaveText("1");
  await expect(page.getByTestId("context-panel-item")).toHaveCount(1);
  await expect(page.getByTestId("context-panel-note-mark")).toHaveAttribute("aria-label", "Note: Check the greeting.");
  await page.getByTestId("context-panel-remove").click();
  await expect(page.getByTestId("context-panel-item")).not.toBeVisible();
  await expect(badge).toHaveCount(0);
  await expect(page.getByTestId("context-tab-empty")).toBeVisible();
});

// Text selected on any page is added from its right-click menu: Add
// Selection to Context offers a note, then puts the text in the basket,
// from the page it was on. Nothing floats over the selection.
test("text selected on a page goes in the basket from its right-click menu", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await shown(page).getByText("Suite home in plain mode").click({ clickCount: 3 });
  await expect(shown(page).getByTestId("selection-context-button")).toHaveCount(0);
  await shown(page).getByText("Suite home in plain mode").click({ button: "right" });
  await page.getByTestId("menu-item-add-to-context").click();
  await page.getByTestId("context-note-add").click();
  await expect(page.getByTestId("context-add-card")).not.toBeVisible();
  await expect(page.getByTestId("dock-context-badge")).toHaveText("1");
  await expect(page.getByTestId("context-panel-item")).toContainText("Suite home in plain mode");
});

// An add can go to a named basket: To → + New basket takes its name. The
// panel then opens that basket's tab with the item. This thread's next prompt
// sends This Thread until Send Next picks the basket: then its tab wears the
// paper plane and the badge counts its items.
test("an add to a new basket is counted once it is sent next", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await page.locator('[data-testid="page-nav-menu"]:visible').click();
  await page.getByTestId("menu-item-page-add-to-context").click();
  await page.locator('[data-testid="context-add-to"][data-value="@new"]').click();
  await page.getByTestId("context-note-new-basket").fill("Greeting review");
  await page.getByTestId("context-note-new-basket").press("Enter");
  await expect(page.getByTestId("context-add-card")).not.toBeVisible();

  const tab = page.locator('[data-testid="context-tab"][data-basket="Greeting review"]');
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("context-panel-item")).toHaveCount(1);
  await expect(shown(page).getByTestId("dock-context-badge")).toHaveCount(0);

  await page.getByTestId("context-send-next").click();
  await expect(tab.getByTestId("context-tab-sending")).toBeVisible();
  await expect(page.getByTestId("dock-context-badge")).toHaveText("1");
});
