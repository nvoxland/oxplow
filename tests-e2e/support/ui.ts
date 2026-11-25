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

/** What a narrow screen needs of a page: nothing reaches past the shown tab's
 *  right edge (cut off, or making the window scroll sideways), except inside
 *  a box that scrolls sideways on purpose. A page scrolls in a box of its own,
 *  so the tab body's own width says nothing: each element is measured. The
 *  boxes that do scroll sideways are returned, for the caller to note. */
export async function expectNoHorizontalOverflow(page: Page): Promise<string[]> {
  const root = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    client: document.documentElement.clientWidth,
  }));
  expect(root.scroll, `the window scrolls sideways (${root.scroll} > ${root.client})`).toBeLessThanOrEqual(root.client);
  const found = await shown(page)
    .first()
    .evaluate((body) => {
      const edge = body.getBoundingClientRect().right + 1;
      const name = (el: Element) => {
        const id = el.getAttribute("data-testid") ?? el.getAttribute("data-node");
        const cls = typeof el.className === "string" && el.className ? `.${el.className.trim().split(/\s+/).join(".")}` : "";
        return `${el.tagName.toLowerCase()}${id ? `[${id}]` : ""}${cls}`;
      };
      const scrolls = (el: Element) => {
        const x = getComputedStyle(el).overflowX;
        return (x === "auto" || x === "scroll") && el.scrollWidth > el.clientWidth + 1;
      };
      const past: string[] = [];
      const boxes: string[] = [];
      const walk = (el: Element) => {
        const box = el.getBoundingClientRect();
        if (box.width === 0 && box.height === 0) return;
        // The code editor lays out huge layers of its own and clips them.
        if (el.classList.contains("monaco-editor")) return;
        if (box.right > edge) past.push(`${name(el)} reaches ${Math.round(box.right)} > ${Math.round(edge - 1)}`);
        if (scrolls(el)) {
          boxes.push(`${name(el)} scrolls ${el.scrollWidth} in ${el.clientWidth}`);
          return; // What is inside scrolls with it.
        }
        for (const child of el.children) walk(child);
      };
      for (const child of body.children) walk(child);
      return { past, boxes };
    });
  expect(found.past, "content reaches past the shown tab's right edge").toEqual([]);
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
