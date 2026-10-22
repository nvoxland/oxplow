/// What a `page:` tab draws comes from its declaration: a `pages:` entry
/// in an extension's manifest (oxplow's own in oxplow-foundation). A page
/// the kit can't draw yet names one of oxplow's built-in pages instead of
/// a lens; `BUILT_IN_PAGES` mirrors the Rust table (and its golden,
/// `crates/oxplow-app/fixtures/built_in_pages.json`), which only shrinks.
import type { Extension, ExtensionPage } from "../tauri-bridge/generated/bindings.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import { extPageUri, pageUri as corePageRef } from "../uri/uri.js";

/// The built-in renderers whose bodies draw on the window's own state
/// (`App.tsx` `appRenderers`): the editor's buffers, the thread's sessions.
export const APP_RENDERERS = [
  "agent-session",
  "commit",
  "editor",
  "file-viewer",
  "lens",
  "metric-detail",
  "wiki-page",
  "work-item",
] as const;

export type AppRendererName = (typeof APP_RENDERERS)[number];

export function isAppRenderer(name: string): name is AppRendererName {
  return (APP_RENDERERS as readonly string[]).includes(name);
}

/// The rest of the built-ins (`builtInPages.tsx`).
export const BUILT_IN_PAGES = [
  "alerts",
  "backlog",
  "board",
  "comments",
  "custom-dashboard",
  "diff",
  "diff-view",
  "duplicate-block",
  "explore-data",
  "external-url",
  "files",
  "git-dashboard",
  "metrics-recorded",
  "resource",
  "settings",
  "terminal",
  "uncommitted-changes",
  "tasks",
] as const;

export type BuiltInPageName = (typeof BUILT_IN_PAGES)[number];

export function isBuiltInPage(name: string): name is BuiltInPageName {
  return (BUILT_IN_PAGES as readonly string[]).includes(name);
}

/// The declared page a `page:` tab names, among the enabled extensions'
/// pages (by its tab id: `page:<id>` for oxplow's, `page:<ext>/<id>` for
/// any other's). `undefined` while the extensions load; `null` when no
/// enabled extension declares it.
export function declaredPage(extensions: Extension[] | null, page: PagePayload): ExtensionPage | null | undefined {
  if (extensions === null) return undefined;
  const pageRef = page.extension ? extPageUri(page.extension, page.page) : corePageRef(page.page);
  for (const e of extensions) {
    if (!e.enabled) continue;
    const found = e.pages.find((p) => p.pageRef === pageRef);
    if (found) return found;
  }
  return null;
}
