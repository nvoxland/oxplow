/**
 * The window's side of renderers (`.context/pages-and-tabs.md` "Which page
 * draws a URI"): the view keys the shell reads, which of the daemon's
 * offered pages a URI's view picks, and the text of a tab. Which pages can
 * draw a URI is the daemon's (`renderers_for`); nothing here decides it.
 */
import type { Page, PageRun, Renderer, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { firstLoader } from "./firstLoader.js";
import { formatUri, parseUri } from "../uri/uri.js";

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

/** The page to draw a URI with, among `renderers` (what the daemon says
 *  can draw it, `useRenderers`): the one `view` names
 *  (`#oxplow.renderer=<extension>/<page>`), else the default, else the
 *  first; null when none can — the caller shows foundation's fallback
 *  (`resource`). A name the view gives that the daemon doesn't offer is
 *  ignored. Which pages those are is decided in the daemon alone
 *  (`pages/renderers.rs`). */
export function pickRenderer(renderers: readonly Renderer[], view: Record<string, unknown>): Renderer | null {
  const named = view[RENDERER_VIEW_KEY];
  return (
    (typeof named === "string" ? renderers.find((r) => r.page === named) : undefined) ??
    renderers.find((r) => r.default) ??
    renderers[0] ??
    null
  );
}

/** What Open With offers for a tab: the renderers to pick among, the one
 *  drawing now and what picking one does (a view change in the tab; the
 *  default renderer is the absent key, so tabs match on it). Undefined
 *  with fewer than two renderers (a route has none). The nav bar's ☰ and
 *  the rail's See also both draw from it. */
export interface OpenWith {
  options: { name: string; title: string }[];
  current: string;
  pick(name: string): void;
}

export function openWithOf(
  renderers: readonly Renderer[] | undefined,
  view: Record<string, unknown> | undefined,
  patchView: ((patch: Record<string, SqlCell | undefined>) => void) | undefined,
): OpenWith | undefined {
  if (!renderers || !patchView) return undefined;
  const current = pickRenderer(renderers, view ?? {});
  if (renderers.length < 2 || !current) return undefined;
  return {
    options: renderers.map((r) => ({ name: r.page, title: r.title })),
    current: current.page,
    pick: (name) => patchView({ [RENDERER_VIEW_KEY]: renderers.find((r) => r.page === name)?.default ? undefined : name }),
  };
}

/** What a renderer's page is once its params change: the URI `uri` with
 *  each of `params` set (one set to nothing dropped; its page's own `uri`
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
 *  route's) and each `{{row.<column>}}` from `row` (its first loader's
 *  first row, once its run has answered), each through its filters, else
 *  — no label, one that fills to nothing, or one that names the row before
 *  there is one — its title. */
export function tabLabel(
  page: Pick<Page, "label" | "title">,
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

/** What a tab says on hover when its label shortens a path to the file's
 *  name (`{{param.path|basename}}`): the whole path. */
export function tabHint(page: Pick<Page, "label" | "title">, params: Record<string, unknown>): string | undefined {
  const named = /\{\{\s*param\.([a-z0-9_]+)\s*\|\s*basename\s*\}\}/.exec(page.label ?? "");
  const v = named ? params[named[1]!] : undefined;
  return typeof v === "string" && v !== "" ? v : undefined;
}

/** The row a page file's `{{row.*}}` reads: its first declared loader's
 *  first row, by column; null before that loader has a row. */
export function labelRow(run: PageRun): Record<string, SqlCell> | null {
  const first = firstLoader(run.page)?.name;
  const result = first ? run.loaders[first]?.result : undefined;
  const row = result?.rows[0];
  if (!result || !row) return null;
  return Object.fromEntries(result.columns.map((c, i) => [c, row[i] ?? null]));
}
