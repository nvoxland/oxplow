/// What a viz reads beyond its own rows: an `expand` column's row opens
/// another *page* (`<ext>/<page>[#<loader>]`) and a lazy tree's branch
/// re-runs the node's page with the lazy param set to the branch's id.
/// `pages/PageTree.tsx` provides them per viz node; outside a page node
/// (none) a row opens nothing and a branch stays closed.
import { createContext, useContext, type ComponentType } from "react";

import type { TabRef } from "../tabs/tabState.js";
import type { VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { HostedData } from "./componentBridge.js";
import type { BranchRun } from "./vizModel.js";

export interface ExpansionProps {
  /** What the column's `expand` names. */
  target: string;
  /** The row's values, by column. */
  params: Record<string, SqlCell>;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
}

export interface VizPages {
  /** Draws what an open `expand` column's row opens. */
  Expansion: ComponentType<ExpansionProps>;
  /** A lazy tree's branch runs, by branch id (`useLazyBranches`'s shape). */
  useBranches(run: VizRun, open: readonly string[], streamId: string | null, maxRows?: number): ReadonlyMap<string, BranchRun>;
  /** What a `custom` viz's component is fed (its page's loaders); null:
   *  the viz's own rows. */
  componentData: HostedData | null;
}

export const VizPagesContext = createContext<VizPages | null>(null);

/** The page reads in force, or null outside a page node. */
export function useVizPages(): VizPages | null {
  return useContext(VizPagesContext);
}
