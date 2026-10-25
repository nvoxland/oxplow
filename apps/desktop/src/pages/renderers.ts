/**
 * Which page renders a URI (`.context/pages-and-tabs.md` "Renderers"): the
 * declared pages whose `for` names its resource (an oxplow URI's
 * `<extension>/<name>`, a web page's scheme) and whose `accepts` hold for
 * its params. The URI's view picks among them (`#oxplow.renderer=
 * <extension>/<page>`); else its resource's default; else the first there
 * is; else none — and the caller shows foundation's fallback (`resource`).
 * Nothing here knows a resource: the declarations say it all.
 */
import type { ExtensionPage, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
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

/** What renders a resource, a page file (`Page`, its `id` already
 *  `<extension>/<slug>`) or a manifest's `pages:` entry (`ExtensionPage`,
 *  its `id` the bare name): the same match rules for both. */
export interface RendererDecl {
  id: string;
  extension: string;
  /** A page file's slug; a `pages:` entry has none. */
  slug?: string;
  for: string[];
  accepts: Partial<Record<string, string>>;
  default: boolean;
}

/** A renderer's name in a view (`<extension>/<page>`). */
export function rendererName(page: RendererDecl): string {
  return `${page.extension}/${page.slug ?? page.id}`;
}

/** Whether `page`'s `accepts` hold for `params`: each named param present,
 *  absent, or matching its pattern. */
function takes(page: RendererDecl, params: Record<string, string>): boolean {
  return Object.entries(page.accepts).every(([key, rule]) => {
    const value = params[key];
    if (rule === "present") return value !== undefined;
    if (rule === "absent") return value === undefined;
    return value !== undefined && new RegExp(rule ?? "").test(value);
  });
}

/** The pages that render `id`, in the order the extensions list them. */
export function renderersFor<P extends RendererDecl>(id: string, pages: readonly P[]): P[] {
  const resource = renderedResourceOf(id);
  if (resource === null) return [];
  const params = parseUri(id)?.params ?? {};
  return pages.filter((p) => p.for.includes(resource) && takes(p, params));
}

/** The page that renders `id` in `view`, or null when no page does. */
export function rendererFor<P extends RendererDecl>(
  id: string,
  pages: readonly P[],
  view: Record<string, unknown>,
): P | null {
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

/** The filters a label applies (`{{param.sha|short}}`): `short` is the first
 *  7 characters (a sha), `basename` the last path segment, `url` a web page's
 *  host and path cut to 40. */
const LABEL_FILTERS: Record<string, (v: string) => string> = {
  short: (v) => v.slice(0, 7),
  basename: (v) => v.replace(/\/+$/, "").split("/").pop() ?? v,
  url: (v) => {
    try {
      const u = new URL(v);
      const text = `${u.host}${u.pathname === "/" ? "" : u.pathname}`;
      return text.length > 40 ? `${text.slice(0, 39)}…` : text;
    } catch {
      return v;
    }
  },
};

/** What a tab of `page` says: its `label` with each `{{param.<name>}}`
 *  filled from `params` (what it shows: a renderer's URI params, a
 *  route's) and each `{{row.<column>}}` from `row` (the one row of the
 *  page's lens, once its run has answered), each through its filters, else
 *  — no label, one that fills to nothing, or one that names the row before
 *  there is one — its title. */
export function tabLabel(
  page: Pick<ExtensionPage, "label" | "title">,
  params: Record<string, unknown>,
  row?: Record<string, unknown> | null,
): string {
  const label = page.label ?? "";
  const reads = /\{\{\s*row\./.test(label);
  if (reads && !row) return page.title;
  const filled = label.replace(/\{\{\s*(param|row)\.([a-z0-9_]+)((?:\s*\|\s*[a-z]+)*)\s*\}\}/g, (_, scope: string, name: string, filters: string) => {
    const v = (scope === "row" ? row : params)?.[name];
    let text = v === null || v === undefined ? "" : String(v);
    for (const f of filters.split("|").map((x) => x.trim()).filter(Boolean)) text = LABEL_FILTERS[f]?.(text) ?? text;
    return text;
  });
  return filled.trim() || page.title;
}

/** Whether `page`'s label names a column of its lens's row, so its tab is
 *  named once the lens has run. */
export function labelReadsRow(page: Pick<ExtensionPage, "label">): boolean {
  return /\{\{\s*row\./.test(page.label ?? "");
}

/** The row a page file's `{{row.*}}` reads: its first declared loader's
 *  first row, by column; null before that loader has a row. */
export function labelRow(run: PageRun): Record<string, SqlCell> | null {
  const first = run.page.loaders[0]?.name;
  const result = first ? run.loaders[first]?.result : undefined;
  const row = result?.rows[0];
  if (!result || !row) return null;
  return Object.fromEntries(result.columns.map((c, i) => [c, row[i] ?? null]));
}
