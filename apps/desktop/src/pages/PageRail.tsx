/// A page file's rail, the one place its controls are
/// (`.context/usability.md` "A page's controls are in its rail"): Filters
/// (its filter params as kit rows, Reset when one is changed), the page's
/// own `rail:` tree, drawn as its view is, Selection (what a row picked)
/// and About this page (its source, and whether it is out of date).
import { isDefault, ParamList } from "../kit/ParamControls.js";
import { RailGroup } from "../kit/RailGroup.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageTree } from "./PageTree.js";
import { RailAbout, RailSelection, type PickedRow } from "./railGroups.js";

export interface PageRailProps {
  run: PageRun;
  runId: string | null;
  /** A filter changed (or was reset), or a row was picked or cleared. */
  onApply(name: string, value: SqlCell, row?: PickedRow): void;
  /** The rows picked for its selection params, by param. */
  picked?: Record<string, PickedRow>;
  /** Its source changed after the deployment that runs. */
  outOfDate?: boolean;
  streamId?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
}

export function PageRail({ run, runId, onApply, picked = NO_PICKED, outOfDate = false, streamId = null, onOpenPage }: PageRailProps) {
  // A resource param is what the page is, a selection what a row picked:
  // neither is an input.
  // No role is a resource, as the server reads it.
  const filters = run.page.params.filter((p) => p.role === "filter");
  const selections = run.page.params.filter((p) => p.role === "selection");
  const changed = filters.filter((p) => !isDefault(p, run.params[p.name] ?? null));
  const reset =
    changed.length > 0
      ? { id: "page-rail-reset", label: "Reset", title: "Put every filter back to its default", run: () => changed.forEach((p) => onApply(p.name, p.default ?? null)) }
      : undefined;
  return (
    <>
      {filters.length > 0 ? (
        <RailGroup title="Filters" action={reset} testId="page-rail-filters">
          <ParamList params={filters} values={run.params} onApply={onApply} streamId={streamId} />
        </RailGroup>
      ) : null}
      {run.rail ? <PageTree run={run} runId={runId} root={run.rail} streamId={streamId} onOpenPage={onOpenPage} onSetParam={onApply} /> : null}
      <RailSelection params={selections} values={run.params} picked={picked} onApply={onApply} testPrefix="page" />
      {onOpenPage ? <RailAbout origin={run.page.origin} outOfDate={outOfDate} onOpenPage={onOpenPage} testPrefix="page" /> : null}
    </>
  );
}

const NO_PICKED: Record<string, PickedRow> = {};
