// Moves a person makes in the shell, shared by specs.
import { expect, type Page } from "@playwright/test";

/** Expand the rail section `id` (`ext:oxplow-bundled/go-to`, `ext:oxplow-bundled/work`) unless it already is — once
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
  // The page is a lens form: it opens once the extensions have loaded, and
  // a shortcut pressed before that goes nowhere, so press until it shows.
  await expect(async () => {
    await page.getByTestId("rail-hud").click({ position: { x: 4, y: 4 } });
    await page.keyboard.press("ControlOrMeta+Shift+N");
    await expect(page.getByTestId("lens-form-fields-title")).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
}

/** Open the launcher and type `query` into it — once it has the focus: it
 *  shows when the window's stream has loaded, and keys typed before that
 *  (or while a late page takes the focus) go nowhere. */
export async function searchLauncher(page: Page, query: string): Promise<void> {
  await page.getByTestId("title-bar-search").click();
  await expect(page.getByPlaceholder(/^Search everything/)).toBeFocused();
  await page.keyboard.type(query);
}

/** Open a page through the launcher: search for `query`, take the first
 *  result — once it is a page, command or search hit whose title starts
 *  with the query (after a command's group), since Enter takes whatever row is first, the results
 *  update after the typing (the pages arrive with the extensions), "Ask
 *  the Agent: <query>" names any query, and a title merely containing it
 *  ("Untitled dashboard" for "Board") is another thing. */
export async function openFromLauncher(page: Page, query: string): Promise<void> {
  await searchLauncher(page, query);
  const first = page.locator(
    '[data-row-index="0"]:is([data-testid^="launcher-page-"], [data-testid^="launcher-command-"], [data-testid^="launcher-hit-"])',
  );
  // A command's row leads with its group ("⌘Tasks›New Agent Session…").
  const title = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  await expect(first).toHaveText(new RegExp(`(?:^|›)\\s*${title}`, "i"));
  await page.keyboard.press("Enter");
}

/** The New Session page (a lens document) the window shows. */
export function newSessionPage(page: Page) {
  return page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid*="page=new-session"]:visible');
}

/** The id of a work item's URI (`tsk42` of `oxplow:oxplow/work_item?id=tsk42&provider=oxplow`). */
export function workItemIdOf(ref: string): string {
  const id = /[?&]id=([^&#]+)/.exec(ref)?.[1];
  if (!id) throw new Error(`\`${ref}\` names no work item`);
  return decodeURIComponent(id);
}

/** The id of a work item's URI (`tsk42` of `oxplow:oxplow/work_item?id=tsk42&provider=oxplow`). */
export function workItemIdOf(ref: string): string {
  const id = /[?&]id=([^&#]+)/.exec(ref)?.[1];
  if (!id) throw new Error(`\`${ref}\` names no work item`);
  return decodeURIComponent(id);
}
