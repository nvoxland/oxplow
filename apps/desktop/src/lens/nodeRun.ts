/// A page run's nodes as the lens viz components take them
/// (`.context/pages-and-tabs.md` "Page files"): a viz node is a lens with
/// no query, its rows its loader's — the same `VizRun` a lens page draws,
/// as the server's `pages::text::node_run` builds it.
import type { VizRun, Node, Page, PageRun, RunNode, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/** No rows: a viz that reads none (a form), or a loader that didn't run. */
export const EMPTY_RESULT: SqlQueryResult = {
  columns: [],
  rows: [],
  truncated: false,
  reads: { models: [], tables: [] } as unknown as SqlQueryResult["reads"],
  freshness: [],
};

/** Viz node `node` of `run` as a lens run; null for any other node. */
export function nodeRunOf(run: PageRun, node: Node): VizRun | null {
  if (node.kind !== "viz") return null;
  return {
    config: node.config,
    params: run.params,
    result: (node.from !== null && run.loaders[node.from]?.result) || EMPTY_RESULT,
  };
}

/** `page`'s node `id`, in its view or its rail; null when it has none. */
export function nodeById(page: Page, id: string): Node | null {
  return find(page.view, id) ?? (page.rail ? find(page.rail, id) : null);
}

function find(at: Node, id: string): Node | null {
  if (at.id === id) return at;
  if (at.kind === "stack" || at.kind === "grid" || at.kind === "switch") {
    for (const c of at.children) {
      const hit = find(c, id);
      if (hit) return hit;
    }
  }
  return null;
}

/** The run that holds node `path` (an embedded page's node is
 *  `<embed>/<id>`, nested as deep as the embeds go) and the node's own id
 *  in it; null when no embed on the way ran. */
export function embedAt(run: PageRun, path: string): { run: PageRun; id: string } | null {
  const slash = path.indexOf("/");
  if (slash < 0) return { run, id: path };
  const embedId = path.slice(0, slash);
  const child = findEmbed(run.tree, embedId) ?? (run.rail ? findEmbed(run.rail, embedId) : null);
  return child ? embedAt(child, path.slice(slash + 1)) : null;
}

function findEmbed(at: RunNode, id: string): PageRun | null {
  if (at.id === id) return at.embed;
  for (const c of at.children) {
    const hit = findEmbed(c, id);
    if (hit) return hit;
  }
  return null;
}
