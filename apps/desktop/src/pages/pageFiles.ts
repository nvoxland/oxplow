/// Page files as the window finds and opens them (`.context/pages-and-tabs.md`
/// "Page files", "The desktop host"): a route's page by its id, the page file
/// a URI's renderer (chosen from the daemon's list) names, the params a
/// tab's URI gives its run, and where a change of them goes (the `?`, or
/// the `#` replaced or pushed).
import type { Extension, Node, Page, PageParam, Renderer, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import { isDefaultValue } from "../viz/vizModel.js";
import { pickRenderer } from "./renderers.js";
import { extPageUri, pageUri, type Params } from "../uri/uri.js";

/** The namespace whose pages are core-named (`page=<slug>`): everything
 *  oxplow ships, as its commands are. */
export const CORE_NAMESPACE = "oxplow";

/** The namespace a page file is named in: its id's first segment. */
export function pageNamespace(page: Page): string {
  return page.id.split("/")[0] ?? page.id;
}

/** The page file a route names among the enabled extensions: by its id,
 *  `<namespace>/<slug>`, a core-named one's namespace `oxplow`. Several
 *  extensions share `oxplow`; the server refuses an id two of them
 *  declare, so an id names one page. `undefined` while the extensions
 *  load; null when none declares it. */
export function pageFileOf(extensions: Extension[] | null, route: PagePayload): Page | null | undefined {
  if (extensions === null) return undefined;
  const id = `${route.namespace ?? CORE_NAMESPACE}/${route.page}`;
  for (const e of extensions) {
    if (!e.enabled) continue;
    const page = (e.pageFiles ?? []).find((p) => p.id === id);
    if (page) return page;
  }
  return null;
}

/** The route a page file opens at (`page=<slug>` for oxplow's, else
 *  `page=<namespace>/<slug>`), with its `?` params and `#` view. */
export function pageFileUri(_extensions: readonly Extension[], page: Page, params: Params = {}, view: Params = {}): string {
  const namespace = pageNamespace(page);
  return namespace === CORE_NAMESPACE
    ? pageUri(page.slug, params, view)
    : extPageUri(namespace, page.slug, params, view);
}

/** Every enabled extension's page files. */
export function enabledPageFiles(extensions: readonly Extension[] | null): Page[] {
  return (extensions ?? []).filter((e) => e.enabled).flatMap((e) => e.pageFiles ?? []);
}

/** Whether a param of this role rides the URI's `#` view (a filter, a
 *  pick, a view param); no role is a resource, which rides the `?`. */
export function ridesView(role: PageParam["role"] | undefined): boolean {
  return (role ?? "resource") !== "resource";
}

/** The params a tab's URI gives a page's run: the `?` params it declares
 *  (a renderer's URI also carries its resource's own, `id`, `provider`,
 *  which a page that doesn't declare them can't take), and those of its
 *  `#` the page declares as a filter, a pick or a view param (never the shell's
 *  `oxplow.*`, a `#node=` landing or a resource param spelled in the `#`). */
export function pageRunParams(
  page: Page,
  resource: Record<string, SqlCell>,
  view: Record<string, SqlCell>,
): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const p of page.params) if (p.name in resource) out[p.name] = resource[p.name] ?? null;
  for (const p of page.params) {
    if (!ridesView(p.role) || !(p.name in view)) continue;
    out[p.name] = view[p.name] ?? null;
  }
  return out;
}

/** The params a switch node turns on: a section of the page, somewhere to
 *  come back from as a pick is. */
function switchParams(page: Page): Set<string> {
  const out = new Set<string>();
  const walk = (n: Node) => {
    if (n.kind === "switch") out.add(n.param);
    if (n.kind === "stack" || n.kind === "grid" || n.kind === "switch") n.children.forEach(walk);
  };
  walk(page.view);
  if (page.rail) walk(page.rail);
  return out;
}

/** A page after `patch` (its rail, a row's pick, a switch): the values
 *  that differ from the defaults, apart as its URI holds them — its
 *  `resource` params (the `?`) and its `view` (the `#`: filters and picks)
 *  — and whether the change is somewhere to come back from (another
 *  resource, a pick set, another section: pushed) or not (a filter:
 *  replaced). The same rule as a page's (`pageViewChange`). */
export function pageViewChange(
  page: Page,
  current: Record<string, SqlCell>,
  patch: Record<string, SqlCell>,
): { resource: Record<string, SqlCell>; view: Record<string, SqlCell>; push: boolean } {
  const param = (name: string) => page.params.find((p) => p.name === name);
  const role = (name: string) => param(name)?.role ?? "resource";
  const switches = switchParams(page);
  const resource: Record<string, SqlCell> = {};
  const view: Record<string, SqlCell> = {};
  for (const [name, value] of Object.entries({ ...current, ...patch })) {
    const p = param(name);
    if (p && isDefaultValue(p, value ?? null)) continue;
    (ridesView(role(name)) ? view : resource)[name] = value ?? null;
  }
  const push = Object.entries(patch).some(
    ([name, value]) => role(name) === "resource" || (value !== null && role(name) === "selection") || switches.has(name),
  );
  return { resource, view, push };
}

/** What draws a route (`oxplow:oxplow/page?page=…`): its page file, or
 *  null when no enabled extension has one. */
export function chooseRoute(extensions: Extension[], route: PagePayload): Page | null {
  return pageFileOf(extensions, route) ?? null;
}

/** Where page file `id` is drawn: the pages that embed it or expand a row
 *  into it, the panels that show it, and the slots it is mounted at — what
 *  a part's tab names instead of drawing it (the daemon's `hosts_of`). */
export function hostsOf(extensions: readonly Extension[], id: string): string[] {
  const out: string[] = [];
  const names = (node: Node): boolean => {
    if (node.kind === "embed" && node.page === id) return true;
    if (node.kind === "viz" && (node.config?.columns ?? []).some((c) => c.expand === id)) return true;
    return (node.children ?? []).some(names);
  };
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pageFiles ?? []) {
      if (names(page.view) || (page.rail ? names(page.rail) : false)) out.push(`page \`${page.id}\``);
    }
    for (const panel of ext.panels ?? []) {
      if ([panel.body, panel.collapsed, panel.count, panel.badge].includes(id)) out.push(`panel \`${panel.id}\``);
    }
    for (const mount of ext.ui?.pageSlots ?? []) {
      if (mount.page === id) out.push(`page \`${mount.host}\` (slot \`${mount.slot}\`)`);
    }
  }
  return out;
}

/** The enabled page file with id `<extension>/<slug>`, or null. */
export function pageFileById(extensions: readonly Extension[], id: string): Page | null {
  return enabledPageFiles(extensions).find((p) => p.id === id) ?? null;
}

/** What draws a resource URI: the page file of the renderer the view
 *  picks among the daemon's `renderers` (`pickRenderer`); null: none does
 *  (or the page left the extensions since), and foundation's `resource`
 *  fallback draws it. */
export function chooseRenderer(extensions: readonly Extension[], renderers: readonly Renderer[], view: Record<string, unknown>): Page | null {
  const picked = pickRenderer(renderers, view);
  return picked ? pageFileById(extensions, picked.page) : null;
}
