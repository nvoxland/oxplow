/**
 * Which page renders a URI (`.context/pages-and-tabs.md` "Renderers"): the
 * declared pages whose `for` names its resource (an oxplow URI's
 * `<extension>/<name>`, a web page's scheme) and whose `accepts` hold for
 * its params. The URI's view picks among them (`#oxplow.renderer=
 * <extension>/<page>`); else its resource's default; else the first there
 * is; else none — and the caller shows foundation's fallback (`resource`).
 * Nothing here knows a resource: the declarations say it all.
 */
import type { ExtensionPage } from "../tauri-bridge/generated/bindings.js";
import { formatUri, parseUri, typeOf } from "../uri/uri.js";

/** The view key that names a URI's renderer (`<extension>/<page>`). */
export const RENDERER_VIEW_KEY = "oxplow.renderer";

/** The view keys the shell reads (`oxplow.*`: the renderer pick) — never
 *  a page's own params. */
export function shellView<T>(view: Record<string, T>): Record<string, T> {
  return Object.fromEntries(Object.entries(view).filter(([k]) => k.startsWith("oxplow.")));
}

/** A view without the shell's keys: what the page itself shows by. */
export function pageView<T>(view: Record<string, T>): Record<string, T> {
  return Object.fromEntries(Object.entries(view).filter(([k]) => !k.startsWith("oxplow.")));
}

/** The schemes a page may render besides oxplow's resources. */
const SCHEMES = ["https", "http"];

/** What renders `id` is chosen by: an oxplow URI's resource, an http(s)
 *  URL's scheme; null for anything else. */
export function renderedResourceOf(id: string): string | null {
  const resource = typeOf(id);
  if (resource !== null) return resource;
  const scheme = /^([a-z][a-z0-9+.-]*):\/\//i.exec(id)?.[1]?.toLowerCase();
  return scheme && SCHEMES.includes(scheme) ? scheme : null;
}

/** A renderer's name in a view (`<extension>/<page>`). */
export function rendererName(page: ExtensionPage): string {
  return `${page.extension}/${page.id}`;
}

/** Whether `page`'s `accepts` hold for `params`: each named param present,
 *  absent, or matching its pattern. */
function takes(page: ExtensionPage, params: Record<string, string>): boolean {
  return Object.entries(page.accepts).every(([key, rule]) => {
    const value = params[key];
    if (rule === "present") return value !== undefined;
    if (rule === "absent") return value === undefined;
    return value !== undefined && new RegExp(rule).test(value);
  });
}

/** The pages that render `id`, in the order the extensions list them. */
export function renderersFor(id: string, pages: readonly ExtensionPage[]): ExtensionPage[] {
  const resource = renderedResourceOf(id);
  if (resource === null) return [];
  const params = parseUri(id)?.params ?? {};
  return pages.filter((p) => p.for.includes(resource) && takes(p, params));
}

/** The page that renders `id` in `view`, or null when no page does. */
export function rendererFor(
  id: string,
  pages: readonly ExtensionPage[],
  view: Record<string, unknown>,
): ExtensionPage | null {
  const candidates = renderersFor(id, pages);
  const named = view[RENDERER_VIEW_KEY];
  return (
    (typeof named === "string" ? candidates.find((p) => rendererName(p) === named) : undefined) ??
    candidates.find((p) => p.default) ??
    candidates[0] ??
    null
  );
}

/** What a renderer's page is once its params change: the URI `uri` with
 *  each of `params` set (one set to nothing dropped; its lens's own `uri`
 *  param is the page, not one of them) and `view` as its `#`. A URI that
 *  isn't an oxplow one has no params to change. */
export function withRenderedParams(
  uri: string,
  params: Record<string, string | number | boolean | null>,
  view: Record<string, string | number | boolean | null>,
): string {
  const u = parseUri(uri);
  if (!u) return uri;
  const next = { ...u.params };
  for (const [k, v] of Object.entries(params)) {
    if (k === "uri") continue;
    if (v === null || v === "") delete next[k];
    else next[k] = String(v);
  }
  const shown: Record<string, string> = {};
  for (const [k, v] of Object.entries(view)) if (v !== null && v !== "") shown[k] = String(v);
  return formatUri({ resource: u.resource, params: next, view: shown });
}

/** What a tab of `page` says: its `label` with each `{{param.<name>}}`
 *  filled from `params` (what it shows: a renderer's URI params, a
 *  route's), else — no label, or one that fills to nothing — its title. */
export function tabLabel(page: ExtensionPage, params: Record<string, unknown>): string {
  const filled = (page.label ?? "").replace(/\{\{\s*param\.([a-z0-9_]+)\s*\}\}/g, (_, name: string) => {
    const v = params[name];
    return v === null || v === undefined ? "" : String(v);
  });
  return filled.trim() || page.title;
}
