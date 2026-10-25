import type { SqlCell } from "../api.js";
import { isDefault, ParamList } from "../kit/ParamControls.js";
import { RailGroup } from "../kit/RailGroup.js";
import { RailAbout, RailSelection, type PickedRow } from "../pages/railGroups.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Lens } from "../tauri-bridge/generated/bindings.js";
import { offeredFilters } from "./lensModel.js";

export type { PickedRow };

export interface LensRailProps {
  lens: Lens;
  values: Record<string, SqlCell>;
  /** The rows picked for its selection params, by param. */
  picked: Record<string, PickedRow>;
  /** Its source changed after the deployment that runs. */
  outOfDate: boolean;
  onApply(name: string, value: SqlCell): void;
  onOpenPage(ref: TabRef): void;
  /** Params that are what the page is, never a control (a renderer's
   *  `uri`). */
  fixed?: readonly string[];
  /** The stream the page runs for: a choice's `options_from` lens runs for it. */
  streamId?: string | null;
  /** A view document's section in view, once known: the rail offers the
   *  filters it declares (`offeredFilters`). */
  viewed?: Lens | null;
}

/**
 * A lens page's rail, the one place its controls are: Filters (its filter
 * params as rows), Selection (what a row click picked, never an input),
 * and About this page (its extension and the source file to edit).
 */
export function LensRail({ lens, values, picked, outOfDate, onApply, onOpenPage, fixed = [], streamId = null, viewed = null }: LensRailProps) {
  const filters = offeredFilters(lens, viewed).filter((p) => !fixed.includes(p.name));
  const selections = lens.params.filter((p) => p.role === "selection");
  const changed = filters.filter((p) => !isDefault(p, values[p.name] ?? null));
  const reset = changed.length > 0 ? { id: "lens-rail-reset", label: "Reset", title: "Put every filter back to its default", run: () => changed.forEach((p) => onApply(p.name, p.default ?? null)) } : undefined;
  return (
    <>
      {filters.length > 0 ? (
        <RailGroup title="Filters" action={reset} testId="lens-rail-filters">
          <ParamList params={filters} values={values} onApply={onApply} streamId={streamId} />
        </RailGroup>
      ) : null}
      <RailSelection params={selections} values={values} picked={picked} onApply={onApply} testPrefix="lens" />
      <RailAbout origin={lens.origin} outOfDate={outOfDate} onOpenPage={onOpenPage} testPrefix="lens" />
    </>
  );
}
