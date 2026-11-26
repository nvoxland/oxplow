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

test("the context line opens the switcher; a new thread made there is listed, and a tap shows it", async ({ page, daemon }) => {
  await load(page, "/?shell=tablet");
  await page.getByTestId("tablet-context").click();
  await expect(page.getByTestId(`switcher-thread-${daemon.thread}`)).toHaveAttribute("aria-current", "true");
  await page.getByTestId(`switcher-add-thread-${daemon.stream}`).click();
  await page.getByTestId(`switcher-new-thread-input-${daemon.stream}`).fill("Tablet thread");
  await page.getByTestId(`switcher-new-thread-input-${daemon.stream}`).press("Enter");
  const made = page.getByTestId("tablet-switcher").getByRole("button", { name: /Tablet thread/ });
  await made.click();
  await expect(page.getByTestId("tablet-switcher")).toBeHidden();
  await expect(page.getByTestId("tablet-context")).toContainText("Tablet thread");
});

test("the tray's panel opens as a sheet, and pinned it stays beside the page", async ({ page }) => {
  await load(page, "/?shell=tablet");
  const first = page.getByTestId("tablet-panels").getByRole("button").first();
  const name = (await first.textContent()) ?? "";
  await first.click();
  await expect(page.getByTestId("tablet-panel-sheet")).toBeVisible();
  await page.getByTestId("tablet-panel-sheet").getByTestId("tablet-panel-pin").click();
  await expect(page.getByTestId("tablet-panel-sheet")).toBeHidden();
  await expect(page.getByTestId("tablet-pinned-panel")).toBeVisible();
  expect(name).toContain((await page.getByTestId("tablet-pinned-panel").getAttribute("aria-label")) ?? "?");
  await page.getByTestId("tablet-pinned-panel").getByTestId("tablet-panel-pin").click();
  await expect(page.getByTestId("tablet-pinned-panel")).toBeHidden();
});

test("the menu offers Layout ▸ Desktop Layout", async ({ page }) => {
  await load(page, "/?shell=tablet");
  await page.getByTestId("tablet-menu").click();
  await page.getByTestId("menu-item-shell.menu.layout").click();
  await expect(page.getByTestId("menu-item-shell.desktop")).toBeVisible();
});
