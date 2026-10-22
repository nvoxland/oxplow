import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Page } from "../tabs/Page.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import { querySql, runLens, type LensRun, type SqlCell } from "../api.js";
import { lensRef, withResourceParams } from "../tabs/pageRefs.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { formatContextMention } from "../agent-context-ref.js";
import { pageView, shellView, withRenderedParams } from "./renderers.js";
import { changedParams, lensViewChange, misplacedViewParams } from "../lens/lensModel.js";
import { useHostedView } from "../lens/hostedView.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "../lens/LensResultView.js";
import { LensRail, type PickedRow } from "../lens/LensRail.js";
import type { PageAction } from "../kit/actions.js";

export interface LensPageProps {
  /** `<extension>/<slug>`. */
  lensId: string;
  /** Its resource params: what its URI's `?` carries (a slot's
   *  `{ effort_id }`). Its filters and picks are its tab's view (the URI's
   *  `#`, `PageNavigation.viewParams`). */
  params?: Record<string, SqlCell>;
  /** The page's name as its host knows it (an extension page's manifest
   *  title); the lens's own title otherwise. */
  title?: string;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
  /** The URI it renders, when it is a renderer's lens (run with `{ uri }`):
   *  the page is that URI, so another value of one of its params is that
   *  resource with the param changed (`withRenderedParams`). */
  renders?: string;
}

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `.oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Its rail is the one
 * place for its controls (`LensRail`): its filters, what a row click
 * picked, and where it comes from; Copy, Add to Agent Context and its
 * own actions are the page's, in the nav bar. See `.context/extensions.md`.
 */
/** What a renderer's lens is run with, never a control: the URI it shows. */
const RENDERED = ["uri"];

export function LensPage({ lensId, params, title: pageTitle, stream, onOpenPage, renders }: LensPageProps) {
  const streamId = stream?.id ?? null;
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  // What it shows is its URI's `?` (`params`), how its tab's view (the
  // `#`): a change goes back through the tab and the page redraws from
  // what it gets. Outside a tab (no `setViewParams`) it keeps them itself.
  const nav = useOptionalPageNavigation();
  const inTab = !!nav?.setViewParams;
  const [own, setOwn] = useState<Record<string, SqlCell>>(params ?? {});
  const resource = params ?? NO_PARAMS;
  // The shell's view keys (`#oxplow.renderer=`) are the tab's, never a
  // lens param.
  const view = pageView(nav?.viewParams ?? NO_PARAMS);
  const fromRef = useMemoObject({ ...resource, ...view });
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
  const pageId = nav?.ref?.id ?? lensRef(lensId, resource, view).id;
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
  // Kept in the entry's history state, so Back and a restart still show the
  // row's facts, not the bare value.
  const [picked, setPicked] = useHistoryState<Record<string, PickedRow>>("picked", NO_PICKED);
  // What the URI carries: the values that differ from the defaults. A
  // filter refines how it's shown (the entry is replaced); a row picked is
  // somewhere to come back from (an entry is pushed); another resource
  // param is another page (pushed).
  const setView = (patch: Record<string, SqlCell>) => {
    if (!inTab || !lens) {
      setOwn({ ...overridesRef.current, ...patch });
      return;
    }
    const change = lensViewChange(lens, overridesRef.current, patch);
    const same = JSON.stringify(sorted(change.resource)) === JSON.stringify(sorted(resource));
    if (same || !nav?.ref) nav!.setViewParams!(change.view, { push: change.push });
    else if (renders) nav.navigate({ id: withRenderedParams(renders, change.resource, { ...shellView(nav.viewParams ?? {}), ...change.view }) });
    else nav.navigate(withResourceParams(nav.ref, change.resource, change.view));
  };
  // A filter or a pick belongs in the URI's `#`: one in its `?` is a link
  // spelled wrong, refused rather than guessed.
  const misplaced = lens && inTab ? misplacedViewParams(lens, resource) : [];
  const setParam = (name: string, value: SqlCell, row?: PickedRow) => {
    setView({ [name]: value });
    const { [name]: _, ...rest } = picked;
    setPicked(row ? { ...rest, [name]: row } : rest);
  };
  // A custom component on its own page sets the lens's params, as the
  // rail does.
  const setViewRef = useRef(setView);
  setViewRef.current = setView;
  const componentView = useHostedView({
    keys: lens?.params.map((p) => p.name) ?? NO_KEYS,
    scope: "lens",
    write: (changes) => setViewRef.current(changes),
  });
  const outOfDate = useOutOfDate(lens?.origin.extension ?? null, lens?.origin.deployment ?? null);
  const rightRail = lens ? (
    <LensRail lens={lens} values={run?.params ?? {}} picked={picked} outOfDate={outOfDate} onApply={setParam} onOpenPage={onOpenPage} fixed={renders ? RENDERED : undefined} />
  ) : undefined;

  // A file or a diff fills the page: its title is the chrome's, its body
  // every pixel the page has.
  const fills = lens?.viz === "code" || lens?.viz === "diff";
  return (
    <Page
      testId="page-lens"
      kind="lens"
      titleInBody={!fills}
      // A dashboard wants every pixel; everything else reads as a document.
      width={lens?.viz === "dashboard" || lens?.viz === "board" || fills ? "full" : "reading"}
      fill={fills}
      actions={actions}
      pin={{ tile: { kind: "lens", lensId, optionsJson: JSON.stringify({ size: "wide" }) }, onOpenPage }}
      rightRail={rightRail}
    >
      <div style={lens?.viz === "dashboard" ? { padding: "var(--space-5)" } : fills ? fillStyle : undefined}>
      {fills ? null : <h1 className="ox-title">{title}</h1>}
      {lens?.description && !fills ? <p className="ox-intro">{lens.description}</p> : null}
      {misplaced.length > 0 ? (
        <div data-testid="lens-error" style={errorStyle}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>This link puts the lens's view in its address.</div>
          <div>
            {misplaced.map((n) => `\`${n}\``).join(", ")} {misplaced.length === 1 ? "is a filter or a pick" : "are filters or picks"} of
            this lens: {misplaced.length === 1 ? "it belongs" : "they belong"} in the URI's <code>#</code>, not its <code>?</code>.
          </div>
        </div>
      ) : null}
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
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} onSetParam={setParam} toolbarTo={setLensActions} componentView={componentView} /> : error ? null : <p>Loading…</p>}
      </div>
    </Page>
  );
}

const NO_PARAMS: Record<string, SqlCell> = {};
const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };

/** `values`' entries in key order: two sets compared by their text. */
function sorted(values: Record<string, SqlCell>): [string, SqlCell][] {
  return Object.entries(values).sort(([a], [b]) => a.localeCompare(b));
}

/** The same object while its contents stay the same, so a merged set of
 *  params doesn't re-run the lens on every render. */
function useMemoObject<T extends object>(value: T): T {
  const ref = useRef(value);
  if (JSON.stringify(ref.current) !== JSON.stringify(value)) ref.current = value;
  return ref.current;
}
const NO_KEYS: string[] = [];
const NO_PICKED: Record<string, PickedRow> = {};

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
