/// A page file's rail, the one place its controls are
/// (`.context/usability.md` "A page's controls are in its rail"): Filters
/// (its filter params as kit rows, Reset when one is changed), then the
/// page's own `rail:` tree, drawn as its view is.
import { isDefault, ParamList } from "../kit/ParamControls.js";
import { RailGroup } from "../kit/RailGroup.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageTree } from "./PageTree.js";

export interface PageRailProps {
  run: PageRun;
  runId: string | null;
  /** A filter changed (or was reset). */
  onApply(name: string, value: SqlCell): void;
  streamId?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
}

export function PageRail({ run, runId, onApply, streamId = null, onOpenPage }: PageRailProps) {
  // A resource param is what the page is, a selection what a row picked:
  // neither is an input.
  const filters = run.page.params.filter((p) => p.role !== "resource" && p.role !== "selection");
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
      {run.rail ? <PageTree run={run} runId={runId} root={run.rail} streamId={streamId} onOpenPage={onOpenPage} /> : null}
    </>
  );
}
