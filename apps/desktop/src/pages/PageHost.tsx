/// A page file, open (`.context/pages-and-tabs.md` "Page files"): the
/// window holds a run of it (`usePageRun`, shared by key and kept fresh by
/// the daemon) and draws it in the page chrome — its label as the title,
/// its whole-page actions and Refresh in the nav bar, its filters and
/// `rail:` tree in the rail, its view as a `PageTree`. New params keep the
/// run on screen until the new one answers (no Loading flash).
import { useEffect, useMemo, useState, type CSSProperties } from "react";

import { CommandConfirm } from "../components/CommandConfirm.js";
import type { PageAction } from "../kit/actions.js";
import { Loading } from "../kit/Loading.js";
import { useLensActions } from "../lens/LensResultView.js";
import { windowPageRuns, usePageRunWith, type PageKey, type PageRuns } from "../pageRuns.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle, useTabActions } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Node, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageRail } from "./PageRail.js";
import { NodeRegistryContext, PageTree, createNodeRegistry, useNodeFocus } from "./PageTree.js";
import { labelRow, tabLabel } from "./renderers.js";
import { useNodeActions } from "./useNodeActions.js";

export interface PageHostProps {
  pageKey: PageKey;
  /** The window's runs (default); a test's fake. */
  runs?: PageRuns;
  /** The node a link lands on (`#node=<id>`). */
  focus?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
}

/** How long a page may take before it says it is reading. */
export const LOADING_AFTER_MS = 150;

export function PageHost({ pageKey, runs, focus = null, onOpenPage }: PageHostProps) {
  const { runId, run, error, stale, setParams, rerun } = usePageRunWith(runs ?? windowPageRuns(), pageKey);
  const streamId = pageKey.streamId;
  const page = run?.page ?? null;
  const title = page ? tabLabel(page, run!.params, labelRow(run!)) : "";
  usePageTitle(title || null);

  const registry = useMemo(createNodeRegistry, []);
  useNodeFocus(registry, focus);

  // The page's own actions bind its params, pressed on the run with no
  // node; a command that asks is confirmed as a lens action's is.
  const target = useMemo(() => ({ lens: { id: page?.id ?? "" }, params: run?.params ?? NO_PARAMS }), [page?.id, run?.params]);
  const pageRunner = useNodeActions(runId, null);
  const actions = useLensActions(target, streamId, pageRunner);
  const offered = (page?.actions ?? []).filter((a) => !a.row && !a.group && !a.edit);
  const pageActions: PageAction[] = [
    // A page re-runs on its own when oxplow's data changes; Refresh is for
    // one that reads outside oxplow.
    ...(page?.refresh ? [{ id: "page-refresh", label: "Refresh", standard: true, icon: "refresh-cw", run: () => void rerun() }] : []),
    ...offered.map((a) => ({
      id: `page-action-${a.id}`,
      label: a.label,
      title: a.command,
      busy: actions.busy === a.id,
      disabled: actions.busy !== null && actions.busy !== a.id,
      prominent: a.prominent,
      submenu: a.submenu ?? undefined,
      icon: a.icon ?? undefined,
      run: () => actions.run(a, null),
    })),
  ];
  useTabActions(pageActions);

  const apply = (name: string, value: SqlCell) => void setParams({ [name]: value });
  const reading = useDelayed(!run && !error, LOADING_AFTER_MS);
  const fills = run ? anyFills(run) : false;
  const full = fills || run?.page.view.kind === "grid";
  const hasRail = !!run && (run.rail !== null || run.page.params.some((p) => p.role !== "resource" && p.role !== "selection"));

  return (
    <Page
      testId="page-host"
      kind="page"
      titleInBody={!fills}
      width={full ? "full" : "reading"}
      fill={fills}
      actions={pageActions}
      rightRail={hasRail ? <PageRail run={run!} runId={runId} onApply={apply} streamId={streamId} onOpenPage={onOpenPage} /> : undefined}
    >
      <NodeRegistryContext.Provider value={registry}>
        <div data-testid="page-file" data-stale={stale ? "true" : undefined} aria-busy={stale || undefined} style={fills ? fillStyle : undefined}>
          {page && !fills ? <h1 className="ox-title">{title}</h1> : null}
          {page?.description && !fills ? <p className="ox-intro">{page.description}</p> : null}
          {actions.pending ? (
            <CommandConfirm
              label={actions.pending.action.label}
              command={actions.pending.action.command}
              details={actions.pending.details}
              onConfirm={actions.confirm}
              onCancel={actions.cancel}
              testIdPrefix="page-action-confirm"
            />
          ) : null}
          {actions.form ? (
            <div className="ox-error" role="alert" data-testid="page-action-form-unsupported">
              {actions.form.action.label} opens a form, which a page file can't show yet.{" "}
              <button type="button" className="ox-link" onClick={actions.closeForm}>
                Close
              </button>
            </div>
          ) : null}
          {error ? (
            <div className="ox-error" role="alert">
              {run ? "This page couldn't run again: " : "This page couldn't run: "}
              {error}
            </div>
          ) : null}
          {run ? (
            <PageTree run={run} runId={runId} streamId={streamId} onOpenPage={onOpenPage} onSetParam={(name, value) => apply(name, value)} />
          ) : reading ? (
            <Loading what="the page" />
          ) : null}
        </div>
      </NodeRegistryContext.Provider>
    </Page>
  );
}

const NO_PARAMS: Record<string, SqlCell> = {};
const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };

/** Whether one of the run's viz nodes fills (a file, a diff: its lens
 *  says): the page gives its body every pixel. */
function anyFills(run: PageRun): boolean {
  const walk = (n: Node): boolean =>
    n.kind === "viz" ? n.lens.fills : n.kind === "stack" || n.kind === "grid" || n.kind === "switch" ? n.children.some(walk) : false;
  return walk(run.page.view);
}

/** `on`, once it has held for `ms`. */
function useDelayed(on: boolean, ms: number): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!on) {
      setShown(false);
      return;
    }
    const t = setTimeout(() => setShown(true), ms);
    return () => clearTimeout(t);
  }, [on, ms]);
  return on && shown;
}
