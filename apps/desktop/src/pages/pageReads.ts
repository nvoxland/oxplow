/// What a page node reads of *other* pages (`.context/pages-and-tabs.md`
/// "The desktop host"): an `expand` column, a lazy tree and a param's
/// `options_from` name a page — `<ext>/<page>` (its first loader) or
/// `<ext>/<page>#<loader>` — whose declaration says which params it takes
/// (the server refuses one it doesn't declare) and whose one-shot run
/// gives a picker its options. Injected, so tests need no daemon.
import { createContext, useContext } from "react";

import { unwrapResult } from "../api.js";
import { firstLoader } from "./firstLoader.js";
import type { OptionsSource } from "../kit/ParamControls.js";
import { commands, type Page, type PageRun, type SqlCell, type SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

export interface PageReads {
  /** A deployed page's declaration. */
  page(id: string): Promise<Page>;
  /** Run a page once (no open run: nothing keeps it fresh). */
  run(page: string, params: Record<string, SqlCell>, streamId: string | null): Promise<PageRun>;
  /** An open run as text, as an agent reads it (Copy). */
  text(runId: string): Promise<string>;
}

export const daemonPageReads: PageReads = {
  page: async (id) => unwrapResult(await commands.getPage(id)) as Page,
  run: async (page, params, streamId) => unwrapResult(await commands.runPage(page, null, params, streamId)) as PageRun,
  text: async (runId) => unwrapResult(await commands.pageText(runId)),
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

/** The loader a ref names in `run` (none named: its page's first
 *  declared loader), or null when the run has no such loader. */
export function namedLoader(run: PageRun, loader: string | null): { name: string; result: SqlQueryResult | null; error: string | null } | null {
  const name = loader ?? firstLoader(run.page)?.name ?? null;
  if (name === null) return null;
  const got = run.loaders[name];
  if (!got) return null;
  return { name, result: got.result ?? null, error: got.error ?? null };
}

/** A choice's `options_from` in a page file: the page it names, run once
 *  with the values it declares; its named loader's rows are the options
 *  (`kit/ParamControls.tsx` `OptionsSource`). */
export function pageOptionsSource(reads: PageReads): OptionsSource {
  return {
    params: async (ref) => (await reads.page(parsePageRef(ref).page)).params.map((p) => p.name),
    list: async (ref, params, streamId) => {
      const { page, loader } = parsePageRef(ref);
      const named = namedLoader(await reads.run(page, params, streamId), loader);
      if (!named) throw new Error(`${ref} has no loader ${loader ?? ""}`.trim());
      if (named.error || !named.result) throw new Error(named.error ?? `${ref} listed nothing`);
      return named.result;
    },
  };
}
