// Moves a person makes in the shell, shared by specs.
import { expect, type Locator, type Page } from "@playwright/test";

import { run, type Daemon } from "./daemon.js";

/** The shown tab's body: the one locator for what the window is showing now.
 *  Every tab's body stays in the DOM, hidden when not shown, and the New
 *  Session tab has forms and buttons of its own, so a page-wide locator can
 *  match a hidden tab's. With `needle`, the shown body only when its tab id
 *  (the page's URI) contains it. */
export function shown(page: Page, needle?: string): Locator {
  const has = needle === undefined ? "" : `[data-testid*=${JSON.stringify(needle)}]`;
  return page.locator(`[data-testid^="center-tab-body-"]${has}:visible`);
}

/** Boxes that may scroll sideways on a narrow screen: a table in its scroll
 *  box, a code block, the terminal. Any other box that does is a failure. */
const SCROLLS_ON_PURPOSE = "table, pre, code, .xterm, .ox-table, [data-scrolls-sideways]";

/** What a narrow screen needs of a page: nothing reaches past the right edge
 *  of its nearest clipping ancestor (`overflow-x: hidden|clip`), past the
 *  shown tab's right edge, or past the window's; and no box scrolls sideways
 *  but those of `SCROLLS_ON_PURPOSE`. The shown tab's body and the page's nav
 *  bar are walked; each element is measured, since a page scrolls in boxes of
 *  its own. Boxes that scroll sideways on purpose are returned, for the
 *  caller to note. With no shown tab body, it returns at once. */
export async function expectNoHorizontalOverflow(page: Page): Promise<string[]> {
  const tab = shown(page);
  if ((await tab.count()) === 0) return [];
  const root = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    client: document.documentElement.clientWidth,
  }));
  expect(root.scroll, `the window scrolls sideways (${root.scroll} > ${root.client})`).toBeLessThanOrEqual(root.client);
  const found = await tab.first().evaluate((body, allowed) => {
    const tabEdge = body.getBoundingClientRect().right + 1;
    const windowEdge = document.documentElement.clientWidth + 1;
    const name = (el: Element) => {
      const id = el.getAttribute("data-testid") ?? el.getAttribute("data-node");
      const cls = typeof el.className === "string" && el.className ? `.${el.className.trim().split(/\s+/).join(".")}` : "";
      return `${el.tagName.toLowerCase()}${id ? `[${id}]` : ""}${cls}`;
    };
    const scrolls = (el: Element) => {
      const x = getComputedStyle(el).overflowX;
      return (x === "auto" || x === "scroll") && el.scrollWidth > el.clientWidth + 1;
    };
    const clips = (el: Element) => ["hidden", "clip"].includes(getComputedStyle(el).overflowX);
    const past: string[] = [];
    const boxes: string[] = [];
    const wrong: string[] = [];
    const walk = (el: Element, edge: number) => {
      const box = el.getBoundingClientRect();
      if (box.width === 0 && box.height === 0) return;
      if (box.right > edge) past.push(`${name(el)} reaches ${Math.round(box.right)} > ${Math.round(edge - 1)}`);
      // The code editor lays out huge layers of its own and clips them: its
      // own box is measured above, what is inside is not.
      if (el.classList.contains("monaco-editor")) return;
      if (scrolls(el)) {
        const note = `${name(el)} scrolls ${el.scrollWidth} in ${el.clientWidth}`;
        if (el.matches(allowed) || el.closest(allowed) || (el.children.length > 0 && [...el.children].every((c) => c.matches(allowed)))) boxes.push(note);
        else wrong.push(note);
        return; // What is inside scrolls with it.
      }
      // What an element clips is cut at its own right edge.
      const inner = clips(el) ? Math.min(edge, box.right + 1) : edge;
      for (const child of el.children) walk(child, inner);
    };
    for (const child of body.children) walk(child, Math.min(tabEdge, windowEdge));
    for (const bar of document.querySelectorAll('[data-testid="page-nav-bar"]')) {
      if (bar.getClientRects().length === 0) continue;
      walk(bar, windowEdge);
    }
    return { past, boxes, wrong };
  }, SCROLLS_ON_PURPOSE);
  expect(found.past, "content reaches past the right edge of the tab, the window or a clipping box").toEqual([]);
  expect(found.wrong, "a box scrolls sideways that is not a table, code block or terminal").toEqual([]);
  return found.boxes;
}

/** Wait until the window takes the daemon's calls: it has loaded its stream
 *  and thread, opened the thread's first tabs and loaded the extensions,
 *  and registered (`data-oxplow-ready` on `<html>`, App.tsx). A navigate
 *  before that is refused, or would be pushed aside by those first tabs,
 *  and a click opens nothing, since no thread is shown yet. */
export async function windowReady(page: Page): Promise<void> {
  if (page.url() === "about:blank") await page.goto("/");
  await expect(page.locator("html[data-oxplow-ready]")).toHaveCount(1);
}

/** Load the app at `path` and wait until the window is ready. */
export async function load(page: Page, path = "/"): Promise<void> {
  await page.goto(path);
  await windowReady(page);
}

/** Open a screen of the shell that is no app window (a host's pair or
 *  home screen) and wait until `testId` shows: it never registers as a
 *  window. */
export async function loadScreen(page: Page, path: string, testId: string): Promise<void> {
  await page.goto(path);
  await expect(page.getByTestId(testId)).toBeVisible();
}

/** Reload the window and wait until it is ready again (a reloaded page is
 *  a new document: not ready until it registers again). */
export async function reload(page: Page): Promise<void> {
  await page.reload();
  await windowReady(page);
}

/** Send the window to `uri` with the bus command `oxplow.window.navigate`
 *  (`extra`: `new_tab`, `replace`), once the window is ready for it. */
export async function navigate(page: Page, daemon: Daemon, uri: string, extra: Record<string, unknown> = {}): Promise<void> {
  await windowReady(page);
  await run(daemon, "oxplow.window.navigate", { uri, ...extra });
}

/** Open the page `uri` the way a link or the agent does (`navigate`, which
 *  pushes into the shown tab; `new_tab: true` opens another), then wait until
 *  the shown tab holds the page. Not through the launcher: typing into it is
 *  the slowest and most focus-sensitive path, and only the launcher's own
 *  specs are about it. */
export async function open(page: Page, daemon: Daemon, uri: string, extra: Record<string, unknown> = {}): Promise<void> {
  await navigate(page, daemon, uri, extra);
  await expect(shown(page, uri.split("#")[0])).toHaveCount(1);
}

/** A custom component's frame, scrolled into view first: a frame below the
 *  fold gets no animation frames, and a click never sees its button as
 *  stable. Found by its `label` (the page-wide frames, such as Integrations),
 *  or, with none, the one `custom-component-frame` under `from` (a page, or
 *  the node of a page that holds it). */
export async function inFrame(from: Page | Locator, label?: string) {
  const selector = label === undefined ? '[data-testid="custom-component-frame"]' : `iframe[aria-label=${JSON.stringify(label)}]`;
  await from.locator(selector).scrollIntoViewIfNeeded();
  return from.frameLocator(selector);
}

/** Expand the rail section `id` (`ext:<extension>/<panel>`) unless it already is — once
 *  the rail shows the person's stored layout (it's `aria-busy` until then),
 *  so the state read is the one the click acts on. */
export async function expandRailSection(page: Page, id: string): Promise<void> {
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const toggle = page.getByTestId(`rail-section-toggle-${id}`);
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
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

/** The New Session page (a page document) the window shows. */
export function newSessionPage(page: Page) {
  return page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?"][data-testid*="page=new-session"]:visible');
}

/** The id of a task's URI (`tsk42` of `oxplow:oxplow/task?id=tsk42&provider=oxtask`). */
export function taskIdOf(ref: string): string {
  const id = /[?&]id=([^&#]+)/.exec(ref)?.[1];
  if (!id) throw new Error(`\`${ref}\` names no task`);
  return decodeURIComponent(id);
}
