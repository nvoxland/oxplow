import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Page } from "../tabs/Page.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import { querySql, runLens, type LensRun, type SqlCell } from "../api.js";
import { lensRef } from "../tabs/pageRefs.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { formatContextMention } from "../agent-context-ref.js";
import { changedParams } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "../lens/LensResultView.js";
import { LensRail, type PickedRow } from "../lens/LensRail.js";
import type { PageAction } from "../kit/actions.js";

export interface LensPageProps {
  /** `<extension>/<slug>`. */
  lensId: string;
  /** Its view: the param values its ref carries (a slot's `{ effort_id }`,
   *  a filter, a row picked). The page draws from them. */
  params?: Record<string, SqlCell>;
  /** The page's ref at other params (`lensRef`, an extension page's ref):
   *  how a change of view goes back to its tab. */
  pageRef?(params: Record<string, SqlCell>): TabRef;
  /** The page's name as its host knows it (an extension page's manifest
   *  title); the lens's own title otherwise. */
  title?: string;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `.oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Its rail is the one
 * place for its controls (`LensRail`): its filters, what a row click
 * picked, and where it comes from; Copy, Add to Agent Context and its
 * own actions are the page's, in the nav bar. See `.context/extensions.md`.
 */
export function LensPage({ lensId, params, pageRef, title: pageTitle, stream, onOpenPage }: LensPageProps) {
  const streamId = stream?.id ?? null;
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Its view is its tab's entry (`tabs/session.ts`): a change goes back
  // through the tab and the page redraws from the ref it gets. Outside a
  // tab (no `replaceRef`) it keeps its view itself.
  const nav = useOptionalPageNavigation();
  const inTab = !!nav?.replaceRef && !!pageRef;
  const [own, setOwn] = useState<Record<string, SqlCell>>(params ?? {});
  const fromRef = params ?? NO_PARAMS;
  const overrides = inTab ? fromRef : own;
  const overridesRef = useRef(overrides);
  overridesRef.current = overrides;
  const viewKey = JSON.stringify(overrides);

  const guard = useRequestGuard();

  const refresh = useCallback(async () => {
    const current = guard.begin();
    try {
      const next = await runLens(lensId, overridesRef.current, streamId);
      if (!current()) return;
      setRun(next);
      setError(null);
    } catch (e) {
      if (current()) setError(e instanceof Error ? e.message : String(e));
    }
  }, [lensId, streamId, guard]);

  // Another lens or stream: its old result mustn't show under the new one.
  useEffect(() => {
    setRun(null);
    setError(null);
  }, [lensId, streamId]);

  useEffect(() => {
    void refresh();
  }, [refresh, viewKey]);

  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

  const lens = run?.lens ?? null;
  const title = pageTitle ?? lens?.title ?? lensId;
  usePageTitle(title);

  // Publish this lens's live params, under the id its tab shows, so an
  // agent asking "what am I looking at" (`get_open_page`) re-runs it with
  // exactly these values.
  const runParams = run?.params;
  const pageId = nav?.ref?.id ?? lensRef(lensId, overrides).id;
  useEffect(() => {
    const store = getPageDetailStore();
    store.publish(pageId, { lensId, params: runParams ?? {} });
    return () => store.publish(pageId, null);
  }, [lensId, pageId, runParams]);

  const improveWithAgent = () => {
    const params = lens ? changedParams(lens, run?.params ?? {}) : {};
    insertIntoAgent(formatContextMention({ kind: "lens", lensId, params }));
  };

  // The lens's own toolbar, handed up by its view for the nav bar.
  const [lensActions, setLensActions] = useState<PageAction[]>([]);
  const actions: PageAction[] = [
    { id: "lens-refresh", label: "Refresh", run: () => void refresh() },
    ...lensActions,
    {
      id: "lens-improve-with-agent",
      label: "Improve with Agent",
      title: "Add this lens to the agent's context so you can ask it to change the lens",
      run: improveWithAgent,
    },
  ];

  // A param set from the rail or by a row click (a column's `select`,
  // with the row it picked for the rail's Selection).
  const [picked, setPicked] = useState<Record<string, PickedRow>>({});
  const setParam = (name: string, value: SqlCell, row?: PickedRow) => {
    // What the ref carries: the values that differ from the defaults.
    const next = lens ? changedParams(lens, { ...overridesRef.current, [name]: value }) : { ...overridesRef.current, [name]: value };
    if (inTab) {
      // A filter refines what's shown (the entry is replaced); a row picked
      // is somewhere to come back from (an entry is pushed).
      const role = lens?.params.find((p) => p.name === name)?.role;
      nav!.replaceRef!(pageRef!(next), { push: role === "selection" && value !== null });
    } else {
      setOwn(next);
    }
    setPicked((p) => {
      const { [name]: _, ...rest } = p;
      return row ? { ...rest, [name]: row } : rest;
    });
  };
  const outOfDate = useOutOfDate(lens?.origin.extension ?? null, lens?.origin.deployment ?? null);
  const rightRail = lens ? (
    <LensRail lens={lens} values={run?.params ?? {}} picked={picked} outOfDate={outOfDate} onApply={setParam} onOpenPage={onOpenPage} />
  ) : undefined;

  return (
    <Page
      testId="page-lens"
      kind="lens"
      titleInBody
      // A dashboard wants every pixel; everything else reads as a document.
      width={lens?.viz === "dashboard" ? "full" : "reading"}
      actions={actions}
      pin={{ tile: { kind: "lens", lensId, optionsJson: JSON.stringify({ size: "wide" }) }, onOpenPage }}
      rightRail={rightRail}
    >
      <div style={lens?.viz === "dashboard" ? { padding: "var(--space-5)" } : undefined}>
      <h1 className="ox-title">{title}</h1>
      {lens?.description ? <p className="ox-intro">{lens.description}</p> : null}
      {error ? (
        <div data-testid="lens-error" style={errorStyle}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>This lens couldn't run.</div>
          <div style={{ fontFamily: "var(--font-mono, monospace)", whiteSpace: "pre-wrap" }}>{error}</div>
          <div style={{ marginTop: 8, color: "var(--text-secondary)" }}>
            Use “Improve with Agent” and ask the agent to fix it; it can check its work with
            <code> validate_extension</code>.
          </div>
        </div>
      ) : null}
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} onSetParam={setParam} toolbarTo={setLensActions} /> : error ? null : <p>Loading…</p>}
      </div>
    </Page>
  );
}

const NO_PARAMS: Record<string, SqlCell> = {};

/** Whether an extension's source changed after the deployment that runs:
 *  its live source hash against the deployed one. False with no
 *  deployment, or when it can't be read. */
function useOutOfDate(extension: string | null, deployment: string | null): boolean {
  const [out, setOut] = useState(false);
  useEffect(() => {
    setOut(false);
    if (!extension || !deployment) return;
    let live = true;
    querySql("SELECT hash FROM v_extension_source WHERE name = ?1", [extension])
      .then((r) => {
        const hash = r.rows[0]?.[0];
        if (live) setOut(typeof hash === "string" && hash !== deployment);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [extension, deployment]);
  return out;
}

const errorStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  margin: "12px 0",
  background: "var(--surface-card)",
};
