import type { LoaderDecl, Page } from "../tauri-bridge/generated/bindings.js";

/** The loader whose first row is "the page's row" (`{{row.x}}`, a
 *  whole-page action's `when`, a count, a page ref with no loader named):
 *  the first one the page's author declared, else the system's `content`
 *  loader of a resource page. The daemon's `Page::first_loader`. */
export function firstLoader(page: Pick<Page, "loaders">): LoaderDecl | undefined {
  return page.loaders.find((l) => l.loader.kind !== "content") ?? page.loaders[0];
}

/** The loader a reference to the page with no `#<loader>` means: a query's
 *  last one (the rows it exists for), else the page's first. The daemon's
 *  `Page::default_loader`. */
export function defaultLoader(page: Pick<Page, "loaders" | "kind">): LoaderDecl | undefined {
  return page.kind === "query" ? page.loaders[page.loaders.length - 1] : firstLoader(page);
}
