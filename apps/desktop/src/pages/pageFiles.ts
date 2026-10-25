/// Page files as the window finds and opens them (`.context/pages-and-tabs.md`
/// "Page files", "The desktop host"): a route's page by its id, a URI's
/// renderer among the page files' `for`, the params a tab's URI gives its
/// run, and where a change of them goes (the `?`, or the `#` replaced or
/// pushed). What the host can't draw yet (`drawsAsFile`) still opens on its
/// `pages:` entry's lens until those parts come.
import type { Extension, ExtensionPage, LensParam, Node, Page, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import { isDefaultValue } from "../lens/lensModel.js";
import { declaredPage } from "./declaredPages.js";
import { RENDERER_VIEW_KEY, rendererFor, rendererName, renderersFor } from "./renderers.js";
import { extPageUri, pageUri, type Params } from "../uri/uri.js";

/** The namespace whose pages are core-named (`page=<slug>`):
 *  oxplow-foundation's, as its commands are. */
export const CORE_NAMESPACE = "oxplow";

/** The page file a route names among the enabled extensions: oxplow's by
 *  its slug, any other's by `<extension>/<slug>`. `undefined` while the
 *  extensions load; null when none declares it. */
export function pageFileOf(extensions: Extension[] | null, route: PagePayload): Page | null | undefined {
  if (extensions === null) return undefined;
  const found: { ext: Extension; page: Page }[] = [];
  for (const e of extensions) {
    if (!e.enabled) continue;
    const core = e.namespace === CORE_NAMESPACE;
    if (route.extension === null ? !core : core || e.name !== route.extension) continue;
    const page = (e.pageFiles ?? []).find((p) => p.slug === route.page);
    if (page) found.push({ ext: e, page });
  }
  // Two extensions share the core namespace (oxplow-foundation and
  // oxplow-bundled), so a core-named slug can name a page of each (a
  // panel's body beside the page it summarizes): the one whose extension
  // also declares that page under `pages:` is the route's.
  const owner = found.find(({ ext, page }) => (ext.pages ?? []).some((d) => d.id === page.slug));
  return (owner ?? found[0])?.page ?? null;
}

/** The route a page file opens at (`page=<slug>` for oxplow's, else
 *  `page=<extension>/<slug>`), with its `?` params and `#` view. */
export function pageFileUri(extensions: readonly Extension[], page: Page, params: Params = {}, view: Params = {}): string {
  const core = extensions.find((e) => e.name === page.extension)?.namespace === CORE_NAMESPACE;
  return core ? pageUri(page.slug, params, view) : extPageUri(page.extension, page.slug, params, view);
}

/** Every enabled extension's page files. */
export function enabledPageFiles(extensions: readonly Extension[] | null): Page[] {
  return (extensions ?? []).filter((e) => e.enabled).flatMap((e) => e.pageFiles ?? []);
}

/** The page files that render a resource (`for:`), in the extensions' order. */
export function pageFileRenderers(extensions: readonly Extension[] | null): Page[] {
  return enabledPageFiles(extensions).filter((p) => p.for.length > 0);
}

/** Whether the page host draws `page` today. A form, a custom component,
 *  a lazy tree, a row that expands, a param whose options another page
 *  reads, or an action that opens a form come with a later step: until
 *  then such a page opens on its `pages:` entry's lens. */
export function drawsAsFile(page: Page): boolean {
  if (page.actions.some((a) => a.form)) return false;
  if (page.params.some((p) => hasOptionsFrom(p))) return false;
  return drawable(page.view) && (page.rail === null || drawable(page.rail));
}

function drawable(n: Node): boolean {
  switch (n.kind) {
    case "stack":
    case "grid":
    case "switch":
      return n.children.every(drawable);
    case "component":
      return false;
    case "viz": {
      const l = n.lens;
      if (l.viz === "form" || l.viz === "custom" || l.form || l.custom) return false;
      if (l.tree?.lazy) return false;
      if (l.columns.some((c) => !!(c as { expand?: string | null }).expand)) return false;
      if (l.actions.some((a) => a.form)) return false;
      if (l.params.some((p) => hasOptionsFrom(p))) return false;
      return true;
    }
    default:
      return true;
  }
}

function hasOptionsFrom(p: LensParam): boolean {
  return !!(p as { options_from?: string | null }).options_from;
}

/** The params a tab's URI gives a page's run: its `?` params, and those
 *  of its `#` the page declares as a filter or a pick (never the shell's
 *  `oxplow.*`, a `#node=` landing or a resource param spelled in the `#`). */
export function pageRunParams(
  page: Page,
  resource: Record<string, SqlCell>,
  view: Record<string, SqlCell>,
): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = { ...resource };
  for (const p of page.params) {
    if ((p.role ?? "resource") === "resource" || !(p.name in view)) continue;
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
 *  replaced). The same rule as a lens page's (`lensViewChange`). */
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
    (role(name) === "resource" ? resource : view)[name] = value ?? null;
  }
  const push = Object.entries(patch).some(
    ([name, value]) => role(name) === "resource" || (value !== null && role(name) === "selection") || switches.has(name),
  );
  return { resource, view, push };
}

/** What draws a tab: a page file, or — only for what has no page file, or
 *  one the host can't draw yet — a manifest's `pages:` entry (the lens
 *  path, removed once every page draws as a file). Both null: nothing
 *  declares it. */
export interface Drawer {
  file: Page | null;
  declared: ExtensionPage | null;
}

function enabledDeclared(extensions: readonly Extension[] | null): ExtensionPage[] {
  return (extensions ?? []).filter((e) => e.enabled).flatMap((e) => e.pages ?? []);
}

/** The `pages:` entry of the same name as `file` (its fallback). */
function declaredTwin(extensions: readonly Extension[] | null, file: Page): ExtensionPage | null {
  return enabledDeclared(extensions).find((p) => rendererName(p) === rendererName(file)) ?? null;
}

/** Whether some enabled extension mounts a *lens* into `file`'s slots
 *  (`ui.slots: [{ slot: <page>.sections, lens }]`, named for the page or
 *  for the lens its `pages:` entry draws): a page file's slot shows only
 *  mounted pages, so until those mounts name pages the page stays on its
 *  lens rather than lose them. */
export function hasLensMounts(extensions: readonly Extension[], file: Page): boolean {
  const twin = declaredTwin(extensions, file);
  const prefixes = [`${file.id}.`, ...(twin?.body.kind === "lens" ? [`${twin.body.lens}.`] : [])];
  return extensions.some((e) => e.enabled && (e.ui?.slots ?? []).some((m) => prefixes.some((p) => m.slot.startsWith(p))));
}

/** Whether the page host draws `file` in this window today. */
function drawsHere(extensions: readonly Extension[], file: Page): boolean {
  return drawsAsFile(file) && !hasLensMounts(extensions, file);
}

/** What draws a route (`oxplow:oxplow/page?page=…`). */
export function chooseRoute(extensions: Extension[], route: PagePayload): Drawer {
  const file = pageFileOf(extensions, route) ?? null;
  if (file && drawsHere(extensions, file)) return { file, declared: null };
  const declared = declaredPage(extensions, route) ?? null;
  if (declared) return { file: null, declared };
  return { file, declared: null };
}

/** What draws a resource URI: the page files' renderer (`for`, `accepts`,
 *  the view's `#oxplow.renderer=`, `default`), else a `pages:` renderer. */
export function chooseRenderer(extensions: Extension[], id: string, view: Record<string, unknown>): Drawer {
  const files = pageFileRenderers(extensions);
  const declared = enabledDeclared(extensions).filter((p) => p.for.length > 0);
  const named = view[RENDERER_VIEW_KEY];
  const file = rendererFor(id, files, view);
  // A view naming a renderer only a `pages:` entry declares (the lens
  // runtime's) picks that one.
  if (typeof named === "string" && (!file || rendererName(file) !== named)) {
    const pick = renderersFor(id, declared).find((p) => rendererName(p) === named);
    if (pick && !renderersFor(id, files).some((p) => rendererName(p) === named)) return { file: null, declared: pick };
  }
  if (file && drawsHere(extensions, file)) return { file, declared: null };
  if (file) {
    const twin = declaredTwin(extensions, file);
    return twin ? { file: null, declared: twin } : { file, declared: null };
  }
  return { file: null, declared: rendererFor(id, declared, view) };
}

/** Open With's choices for `id`: every page that renders it, page files
 *  first, each name once. */
export function openWithOptions(extensions: readonly Extension[] | null, id: string): { name: string; title: string }[] {
  const out = new Map<string, string>();
  for (const p of renderersFor(id, pageFileRenderers(extensions))) out.set(rendererName(p), p.title);
  for (const p of renderersFor(id, enabledDeclared(extensions).filter((d) => d.for.length > 0))) {
    if (!out.has(rendererName(p))) out.set(rendererName(p), p.title);
  }
  return [...out].map(([name, title]) => ({ name, title }));
}
