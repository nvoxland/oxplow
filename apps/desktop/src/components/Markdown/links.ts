/**
 * What a link in markdown does, the same in the viewer and the editor:
 * a click opens it (Cmd/Ctrl-click or middle-click in a new tab), and a
 * right-click offers Open, Open in new tab and a Copy of what it names.
 * A page link opens through the page's navigation (`openRef`); a web
 * link opens in the app's web page when the allowlist takes it, else in
 * the browser.
 */
import { classifyExternalUrl } from "../../external-url-allowlist.js";
import type { MenuItem } from "../../menu.js";
import type { NavigateOptions } from "../../tabs/PageNavigationContext.js";
import { externalUrlRef } from "../../tabs/pageRefs.js";
import { workItemLabel } from "../../workItemRef.js";
import { linkRef, parseMarkdownLink } from "./MarkdownView.js";

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
  window.open(href, "_blank", "noopener,noreferrer");
}

/** Follow `href` as a click does. False when it isn't a link to follow
 *  (an in-page anchor, nothing): the caller lets the click through. */
export function followLink(href: string, open: OpenRef, newTab: boolean): boolean {
  const parsed = parseMarkdownLink(href);
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

/** The right-click menu of a link to `href`. */
export function linkMenu(href: string, open: OpenRef): MenuItem[] {
  const parsed = parseMarkdownLink(href);
  if (parsed.kind === "external") {
    const items: MenuItem[] = [];
    if (open && classifyExternalUrl(href).ok) {
      items.push({ id: "open-in-app", label: "Open in app", enabled: true, run: () => openExternal(href, open) });
    }
    items.push({ id: "open-ext", label: "Open in browser", enabled: true, run: () => void window.open(href, "_blank", "noopener,noreferrer") });
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
    case "directory":
      items.push(copy("copy-path", "Copy path", parsed.path));
      break;
    case "commit":
      items.push(copy("copy-sha", "Copy SHA", parsed.sha));
      break;
    case "work_item":
      items.push(copy("copy-id", "Copy id", workItemLabel(parsed.ref)));
      break;
    case "internal":
    case "ref":
      if (target) items.push(copy("copy-link", "Copy link", target));
      break;
  }
  return items;
}
