// Moves a person makes in the shell, shared by specs.
import { expect, type Page } from "@playwright/test";

/** Expand the rail section `id` (`core:work`) unless it already is — once
 *  the rail shows the person's stored layout (it's `aria-busy` until then),
 *  so the state read is the one the click acts on. */
export async function expandRailSection(page: Page, id: string): Promise<void> {
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const toggle = page.getByTestId(`rail-section-toggle-${id}`);
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
}

/** Open the New Task page the way a person does: ⇧⌘N, focus outside a
 *  text field. */
export async function openNewTask(page: Page): Promise<void> {
  await page.getByTestId("rail-hud").click({ position: { x: 4, y: 4 } });
  await page.keyboard.press("ControlOrMeta+Shift+N");
  await expect(page.getByTestId("tasks-title")).toBeVisible();
}

/** Open a page through the launcher: search for `query`, take the first
 *  result — once it is the query's, since Enter takes whatever row is
 *  first, and the results update after the typing. */
export async function openFromLauncher(page: Page, query: string): Promise<void> {
  await page.getByTestId("rail-search").click();
  await page.keyboard.type(query);
  await expect(page.locator('[data-row-index="0"]')).toContainText(query, { ignoreCase: true });
  await page.keyboard.press("Enter");
}
