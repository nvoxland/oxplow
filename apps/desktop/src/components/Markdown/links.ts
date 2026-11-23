/**
 * What a link in markdown does, the same in the viewer and the editor:
 * a click opens it (Cmd/Ctrl-click or middle-click in a new tab), and a
 * right-click offers Open, Open in new tab, Open In ▸ (the browser, the
 * file manager, a terminal: `openIn.ts`) and a Copy of what it names.
 * A page link opens through the page's navigation (`openRef`); a web
 * link opens in the app's web page when the allowlist takes it, else in
 * the system browser.
 */
import { classifyExternalUrl } from "../../external-url-allowlist.js";
import type { MenuItem } from "../../menu.js";
import type { NavigateOptions } from "../../tabs/PageNavigationContext.js";
import { externalUrlRef } from "../../tabs/pageRefs.js";
import { openInBrowser, openInForRef, openInForWorktreePath } from "../../openIn.js";
import { WORKING } from "../../revision.js";
import { parseUri } from "../../uri/uri.js";
import { recordLabel, recordOf } from "../../recordStores.js";
import { linkRef, parseMarkdownLink } from "./markdownLinks.js";

/** How a link opens a page: the page navigation's `openRef`, or none
 *  (outside a page nothing opens in a tab). */
export type OpenRef = ((ref: string, how?: NavigateOptions) => void) | null;

/** Open the web link `href`: in the app when it may be, else the browser. */
export function openExternal(href: string, open: OpenRef, newTab = true): void {
  const verdict = classifyExternalUrl(href);
  if (verdict.ok && open) {
    open(externalUrlRef(verdict.url).id, { newTab });
    return;
  }
  openInBrowser(href);
}

/** Follow `href` as a click does. False when it isn't a link to follow
 *  (an in-page anchor, nothing): the caller lets the click through. `base`
 *  is the URI of the file the markdown came from, which a relative link
 *  resolves against. */
export function followLink(href: string, open: OpenRef, newTab: boolean, base: string | null = null): boolean {
  const parsed = parseMarkdownLink(href, base);
  if (parsed.kind === "anchor" || parsed.kind === "empty") return false;
  if (parsed.kind === "broken") return true;
  if (parsed.kind === "external") {
    openExternal(href, open, newTab);
    return true;
  }
  const target = linkRef(parsed);
  if (target && open) open(target, { newTab });
  return true;
}

const copy = (id: string, label: string, text: string): MenuItem => ({
  id,
  label,
  enabled: true,
  run: () => {
    void navigator.clipboard.writeText(text).catch(() => {});
  },
});

/** The right-click menu of a link to `href` (`base` as `followLink`'s). */
export function linkMenu(href: string, open: OpenRef, base: string | null = null): MenuItem[] {
  const parsed = parseMarkdownLink(href, base);
  if (parsed.kind === "external") {
    const items: MenuItem[] = [];
    const elsewhere = /^https?:\/\//i.test(href) ? openInForRef(href) : null;
    if (elsewhere) items.push(elsewhere);
    else items.push({ id: "open-ext", label: "Open in browser", enabled: true, run: () => openInBrowser(href) });
    items.push(copy("copy", "Copy link", href));
    return items;
  }
  const target = linkRef(parsed);
  const items: MenuItem[] =
    target && open
      ? [
          { id: "open", label: "Open", enabled: true, run: () => open(target, { newTab: false }) },
          { id: "open-new", label: "Open in new tab", enabled: true, run: () => open(target, { newTab: true }) },
        ]
      : [];
  switch (parsed.kind) {
    case "file":
    case "directory": {
      // Only the working copy on the local filesystem is on disk: a URI
      // href says so itself (`localTarget`), a relative one by what it
      // resolved to.
      const onDisk = parsed.kind === "file" && (parsed.version ?? WORKING) === WORKING && !parsed.filesystem;
      const elsewhere = parseUri(href) ? openInForRef(href) : onDisk ? openInForWorktreePath("file", parsed.path) : null;
      if (elsewhere) items.push(elsewhere);
      items.push(copy("copy-path", "Copy path", parsed.path));
      break;
    }
    case "commit":
      items.push(copy("copy-sha", "Copy SHA", parsed.sha));
      break;
    case "ref":
      if (recordOf(parsed.ref)) items.push(copy("copy-id", "Copy id", recordLabel(parsed.ref)));
      if (target) items.push(copy("copy-link", "Copy link", target));
      break;
  }
  return items;
}
