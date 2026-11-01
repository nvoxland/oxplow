/// What a page node reads of *other* pages (`.context/pages-and-tabs.md`
/// "The desktop host"): an `expand` column and a lazy tree name a page —
/// `<ext>/<page>` (its default loader) or `<ext>/<page>#<loader>` — run
/// once; a choice's `options_from` is resolved by the daemon
/// (`page_options`: the query, or the page's own loader, with the params it
/// declares). Injected, so tests need no daemon.
import { createContext, useContext } from "react";

import { unwrapResult } from "../api.js";
import { defaultLoader } from "./firstLoader.js";
import type { OptionsSource } from "../kit/ParamControls.js";
import { commands, type Page, type PageRun, type SqlCell, type SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

export interface PageReads {
  /** A deployed page's declaration. */
  page(id: string): Promise<Page>;
  /** Run a page once (no open run: nothing keeps it fresh). */
  run(page: string, params: Record<string, SqlCell>, streamId: string | null): Promise<PageRun>;
  /** An open run as text, as an agent reads it (Copy). */
  text(runId: string): Promise<string>;
  /** A choice param's options: what its `options_from` lists for the
   *  page's current values (the daemon picks the ones the source declares). */
  options(page: string, param: string, values: Record<string, SqlCell>, streamId: string | null): Promise<SqlQueryResult>;
}

export const daemonPageReads: PageReads = {
  page: async (id) => unwrapResult(await commands.getPage(id)) as Page,
  run: async (page, params, streamId) => unwrapResult(await commands.runPage(page, null, params, streamId)) as PageRun,
  text: async (runId) => unwrapResult(await commands.pageText(runId)),
  options: async (page, param, values, streamId) => unwrapResult(await commands.pageOptions(page, param, values, streamId)) as SqlQueryResult,
};

export const PageReadsContext = createContext<PageReads>(daemonPageReads);

export function usePageReads(): PageReads {
  return useContext(PageReadsContext);
}

/** A page ref split: the page and the loader it names (none: its first). */
export function parsePageRef(ref: string): { page: string; loader: string | null } {
  const hash = ref.indexOf("#");
  return hash < 0 ? { page: ref, loader: null } : { page: ref.slice(0, hash), loader: ref.slice(hash + 1) || null };
}

/** The values of `values` that `page` declares as params: what a run of
 *  it may be given. */
export function declaredParams(page: Pick<Page, "params">, values: Record<string, SqlCell>): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const p of page.params) if (p.name in values) out[p.name] = values[p.name] ?? null;
  return out;
}

/** The loader a ref names in `run` (none named: its page's default
 *  loader — a query's last, else the first), or null when the run has no
 *  such loader. */
export function namedLoader(run: PageRun, loader: string | null): { name: string; result: SqlQueryResult | null; error: string | null } | null {
  const name = loader ?? defaultLoader(run.page)?.name ?? null;
  if (name === null) return null;
  const got = run.loaders[name];
  if (!got) return null;
  return { name, result: got.result ?? null, error: got.error ?? null };
}

/** A choice's `options_from` in a page file: the daemon resolves it (a
 *  query, or the page's own loader) for the page's values
 *  (`kit/ParamControls.tsx` `OptionsSource`). */
export function pageOptionsSource(reads: PageReads): OptionsSource {
  return {
    list: (page, param, values, streamId) => reads.options(page, param, values, streamId),
  };
}
