/// A page file's rail, the one place its controls are
/// (`.context/usability.md` "A page's controls are in its rail"): Filters
/// (its filter params as kit rows, Reset when one is changed), the page's
/// own `rail:` tree, drawn as its view is, Selection (what a row picked)
/// and About this page (its source, and whether it is out of date).
import { useMemo } from "react";

import { isDefault, OptionsSourceContext, ParamList } from "../kit/ParamControls.js";
import { RailGroup } from "../kit/RailGroup.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageTree } from "./PageTree.js";
import { pageOptionsSource, usePageReads } from "./pageReads.js";
import { RailSelection, type PickedRow } from "./railGroups.js";

export interface PageRailProps {
  run: PageRun;
  runId: string | null;
  /** A filter changed (or was reset), or a row was picked or cleared. */
  onApply(name: string, value: SqlCell, row?: PickedRow): void;
  /** The rows picked for its selection params, by param. */
  picked?: Record<string, PickedRow>;
  streamId?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
}

/** A page whose view is a switch: its sections (each child, by its
 *  title), the param that picks one and the one in view. Null for any
 *  other page. */
export function switchSections(run: PageRun): { param: string; sections: { id: string; title: string }[]; current: string | null } | null {
  const view = run.page.view;
  if (view.kind !== "switch") return null;
  return {
    param: view.param,
    sections: view.children.map((c) => ({ id: c.id, title: c.title ?? c.id })),
    current: run.tree.children[0]?.id ?? null,
  };
}

export function PageRail({ run, runId, onApply, picked = NO_PICKED, streamId = null, onOpenPage }: PageRailProps) {
  // A resource param is what the page is, a selection what a row picked:
  // neither is an input. A switch's param is its sections, drawn as links.
  // No role is a resource, as the server reads it.
  const sw = switchSections(run);
  const filters = run.page.params.filter((p) => p.role === "filter" && p.name !== sw?.param);
  // A filter's `options_from` names a page here, not a query.
  const reads = usePageReads();
  const options = useMemo(() => pageOptionsSource(reads), [reads]);
  const selections = run.page.params.filter((p) => p.role === "selection");
  const changed = filters.filter((p) => !isDefault(p, run.params[p.name] ?? null));
  const reset =
    changed.length > 0
      ? { id: "page-rail-reset", label: "Reset", title: "Put every filter back to its default", run: () => changed.forEach((p) => onApply(p.name, p.default ?? null)) }
      : undefined;
  return (
    <>
      {sw ? (
        // The page's sections, each a link to it; the one in view marked.
        <RailGroup title={run.page.title} testId="page-sections">
          <nav aria-label={run.page.title} className="ox-contents">
            {sw.sections.map((o) => (
              <button
                key={o.id}
                type="button"
                className="ox-contents-item"
                data-testid={`page-section-${o.id}`}
                aria-current={o.id === sw.current ? "page" : undefined}
                data-active={o.id === sw.current ? "true" : undefined}
                onClick={() => onApply(sw.param, o.id)}
              >
                {o.title}
              </button>
            ))}
          </nav>
        </RailGroup>
      ) : null}
      {filters.length > 0 ? (
        <RailGroup title="Filters" action={reset} testId="page-rail-filters">
          <OptionsSourceContext.Provider value={options}>
            <ParamList params={filters} values={run.params} onApply={onApply} streamId={streamId} />
          </OptionsSourceContext.Provider>
        </RailGroup>
      ) : null}
      {run.rail ? <PageTree run={run} runId={runId} root={run.rail} streamId={streamId} onOpenPage={onOpenPage} onSetParam={onApply} /> : null}
      <RailSelection params={selections} values={run.params} picked={picked} onApply={onApply} testPrefix="page" />
    </>
  );
}

const NO_PICKED: Record<string, PickedRow> = {};
