import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// A page's ☰ → Add to Context offers a note, then puts the page in the
// selected thread's basket: the dock's Context icon counts it, its panel
// lists it with the note, and the × takes it out again.
test("a page added from its ☰ is counted in the dock and removed from its panel", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await page.locator('[data-testid="page-nav-menu"]:visible').click();
  await page.getByTestId("menu-item-page-add-to-context").click();
  await page.getByTestId("context-note-field").fill("Check the greeting.");
  await page.getByTestId("context-note-add").click();
  await expect(page.getByTestId("context-note-prompt")).not.toBeVisible();

  const badge = page.getByTestId("dock-context-badge");
  await expect(badge).toHaveText("1");

  await page.getByTestId("dock-context").click();
  await expect(page.getByTestId("context-panel-item")).toHaveCount(1);
  await expect(page.getByTestId("context-panel-note")).toHaveText("Check the greeting.");
  await page.getByTestId("context-panel-remove").click();
  await expect(page.getByTestId("context-panel-item")).not.toBeVisible();
  await expect(badge).toHaveCount(0);
  await expect(page.getByTestId("context-panel-empty")).toBeVisible();
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
  await expect(page.getByTestId("context-note-prompt")).not.toBeVisible();
  await expect(page.getByTestId("dock-context-badge")).toHaveText("1");
  await page.getByTestId("dock-context").click();
  await expect(page.getByTestId("context-panel-item")).toContainText("Suite home in plain mode");
});

// An add can go to a named basket: To → New basket… takes its name. The
// basket is listed under Baskets, collapsed, and not counted for this
// thread's next prompt until it is attached; then it shows under This
// Thread and the badge counts it.
test("an add to a new basket is counted once the basket is attached", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await page.locator('[data-testid="page-nav-menu"]:visible').click();
  await page.getByTestId("menu-item-page-add-to-context").click();
  await page.getByTestId("context-note-basket").selectOption("@new");
  await page.getByTestId("context-note-new-basket").fill("Greeting review");
  await page.getByTestId("context-note-new-basket").press("Enter");
  await expect(page.getByTestId("context-note-prompt")).not.toBeVisible();

  await page.getByTestId("dock-context").click();
  const basket = page.locator('[data-testid="context-basket"][data-basket="Greeting review"]');
  await expect(basket).toContainText("Greeting review");
  await expect(page.getByTestId("context-panel-item")).not.toBeVisible();
  await expect(shown(page).getByTestId("dock-context-badge")).toHaveCount(0);

  await page.getByTestId("context-basket-attach").click();
  await expect(page.getByTestId("context-basket-attached")).toContainText("Greeting review");
  await expect(page.getByTestId("dock-context-badge")).toHaveText("1");
});
