/// What a viz reads beyond its own rows, when a page file draws it: an
/// `expand` column's row opens another *page* (`<ext>/<page>[#<loader>]`)
/// and a lazy tree's branch re-runs the node's page with the lazy param
/// set to the branch's id. A lens reads lenses for both (the default,
/// none here); `pages/PageTree.tsx` provides the page ones per viz node,
/// so the same viz components draw both.
import { createContext, useContext, type ComponentType } from "react";

import type { TabRef } from "../tabs/tabState.js";
import type { LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { BranchRun } from "./lensModel.js";

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
  useBranches(run: LensRun, open: readonly string[], streamId: string | null, maxRows?: number): ReadonlyMap<string, BranchRun>;
}

export const VizPagesContext = createContext<VizPages | null>(null);

/** The page reads in force, or null: a lens's own. */
export function useVizPages(): VizPages | null {
  return useContext(VizPagesContext);
}
