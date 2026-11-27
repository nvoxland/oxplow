// The tablet shell draws the same window: its context line, the thread's
// tabs and the page, with no desktop chrome.
import { expect, test } from "../../support/fixtures.js";
import { closeTab, load, open, shown, tabOf } from "../../support/ui.js";

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

const second = "oxplow:oxplow/page?page=e2e/second";
const overview = "oxplow:oxplow/page?page=e2e/overview";

test("a thread is renamed from the switcher's menu", async ({ page, daemon }) => {
  await load(page, "/?shell=tablet");
  await page.getByTestId("tablet-context").click();
  await page.getByTestId(`switcher-thread-${daemon.thread}`).click({ button: "right" });
  await page.getByTestId("menu-item-thread.rename").click();
  const input = page.getByTestId("switcher-rename-input");
  await input.fill("Renamed on a tablet");
  await input.press("Enter");
  await expect(page.getByTestId(`switcher-thread-${daemon.thread}`)).toContainText("Renamed on a tablet");
  await expect(page.getByTestId("tablet-context")).toContainText("Renamed on a tablet");
});

test("the menu's File, Edit, View and Layout submenus open", async ({ page }) => {
  await load(page, "/?shell=tablet");
  for (const group of ["file", "edit", "view", "layout"]) {
    await page.getByTestId("tablet-menu").click();
    const entry = page.getByTestId(`menu-item-shell.menu.${group}`);
    await expect(entry).toBeVisible();
    await entry.click();
    // The submenu draws at least one item of its own, a layout choice for Layout.
    await expect(page.locator('[data-testid^="menu-item-"]:not([data-testid^="menu-item-shell.menu."])').first()).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(entry).not.toBeVisible();
  }
});

test("a tab closes from its ×", async ({ page, daemon }) => {
  await load(page, "/?shell=tablet");
  await open(page, daemon, home);
  await open(page, daemon, second, { new_tab: true });
  await expect(tabOf(page, second)).toHaveAttribute("aria-selected", "true");
  await closeTab(page, second);
  await expect(tabOf(page, second)).toHaveCount(0);
  await expect(tabOf(page, home)).toBeVisible();
});

test("a tab's menu moves it left and right", async ({ page, daemon }) => {
  await load(page, "/?shell=tablet");
  await open(page, daemon, home);
  await open(page, daemon, second, { new_tab: true });
  await open(page, daemon, overview, { new_tab: true });
  const order = () => page.getByTestId("tablet-tabs").locator("[role=tab]").evaluateAll((tabs) => tabs.map((t) => t.getAttribute("data-testid")?.replace("tablet-tab-", "")));
  const before = (await order()) as string[];
  expect(before.slice(-3)).toEqual([home, second, overview]);
  await tabOf(page, overview).click({ button: "right" });
  await page.getByTestId("menu-item-tab.move-left").click();
  await expect.poll(async () => (await order()).slice(-3)).toEqual([home, overview, second]);
  await tabOf(page, overview).click({ button: "right" });
  await page.getByTestId("menu-item-tab.move-right").click();
  await expect.poll(async () => (await order()).slice(-3)).toEqual([home, second, overview]);
});

test("the tray's sheet has an Open link that opens the panel's page", async ({ page }) => {
  await load(page, "/?shell=tablet");
  await page.getByTestId("tablet-panels").getByRole("button").first().click();
  const sheet = page.getByTestId("tablet-panel-sheet");
  await expect(sheet).toBeVisible();
  await sheet.getByTestId("open-page-link").click();
  await expect(sheet).toBeHidden();
  await expect(shown(page)).toHaveCount(1);
});

test("a dock panel (Alerts) opens from the tray at the window's width", async ({ page }) => {
  await load(page, "/?shell=tablet");
  await page.getByTestId("dock-alerts").click();
  const dock = page.getByTestId("dock-panel");
  await expect(dock).toBeVisible();
  const box = await dock.boundingBox();
  const width = page.viewportSize()!.width;
  expect(box!.width).toBeGreaterThan(width * 0.9);
});
