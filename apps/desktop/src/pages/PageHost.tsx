/// A page file, open (`.context/pages-and-tabs.md` "Page files"): the
/// window holds a run of it (`usePageRun`, shared by key and kept fresh by
/// the daemon) and draws it in the page chrome — its label as the title,
/// its whole-page actions and Refresh in the nav bar, its filters and
/// `rail:` tree in the rail, its view as a `PageTree`. New params keep the
/// run on screen until the new one answers (no Loading flash).
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { CommandConfirm } from "../components/CommandConfirm.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { contextNodeProps } from "../components/Comments/contextNodes.js";
import { requestCommentReveal } from "../comment-reveal-bus.js";
import { PageCommentsContext, type PageComments } from "../viz/pageComments.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { Contents } from "../viz/Contents.js";
import { VizStreamContext } from "../viz/VizStream.js";
import type { Stream } from "../api.js";
import { identityOf } from "../uri/uri.js";
import { useOutOfDate, type PickedRow } from "./railGroups.js";
import type { PageAction } from "../kit/actions.js";
import { Loading } from "../kit/Loading.js";
import { ActionForm, useVizActions } from "../viz/VizView.js";
import { FormSourceContext } from "../viz/formSource.js";
import { offeredWholePage } from "../viz/vizActions.js";
import { VizActionRunnerContext } from "../viz/vizActionRunner.js";
import { usePageRunsStore, usePageRunWith, type PageKey, type PageRuns } from "../pageRuns.js";
import { Page } from "../tabs/Page.js";
import { useOptionalPageNavigation, usePageTitle, useTabActions } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Node, PageParam, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageRail, switchSections } from "./PageRail.js";
import { ComponentPage, type ComponentPageHost } from "./PageComponent.js";
import type { BridgeHost } from "../viz/ComponentFrame.js";
import type { FrameAction } from "../viz/componentBridge.js";
import { NodeRegistryContext, PageTree, createNodeRegistry, useNodeFocus } from "./PageTree.js";
import { labelRow, tabLabel } from "./renderers.js";
import { useNodeActions, useNodeForms } from "./useNodeActions.js";
import { SaveViewForm } from "./SaveViewForm.js";
import { definitionUri } from "./viewActions.js";
import { PageViewContext } from "./PageViewContext.js";
import { usePageReads } from "./pageReads.js";
import { recordOpError } from "../opErrors.js";
import { HostedViewContext, useHostedView } from "../viz/hostedView.js";

export interface PageHostProps {
  pageKey: PageKey;
  /** The window's runs (default); a test's fake. */
  runs?: PageRuns;
  /** The node a link lands on (`#node=<id>`). */
  focus?: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  /** In a tab, a change of params (a filter, a pick, a switch) goes here:
   *  the tab's entry is what the page shows, and the run follows it
   *  (`PageFileView`). Outside a tab the open run takes the new params. */
  onChange?(patch: Record<string, SqlCell>): void;
  /** The resource URI it renders (a renderer's): what links to it and
   *  what it links to are listed. */
  renders?: string | null;
  /** The stream it runs for, whole: what a viz that needs more than its
   *  id reads (the editor: the worktree its files are under). */
  stream?: Stream | null;
  /** Keep a shown view's exact source in its tab's history state. */
  onSpecYaml?(yaml: string): void;
}

const NO_ROWS = { columns: [], rows: [] };

/** How long a page may take before it says it is reading. */
export const LOADING_AFTER_MS = 150;

export function PageHost({ pageKey, runs, focus = null, onOpenPage, onChange, renders = null, stream = null, onSpecYaml }: PageHostProps) {
  const store = usePageRunsStore();
  const state = usePageRunWith(runs ?? store, pageKey);
  const { error, setParams, rerun } = state;
  // A tab re-keys its run when its view changes: the last run of the same
  // page stays on screen, marked, until the new one answers.
  const held = useRef<{ runId: string | null; run: PageRun } | null>(null);
  if (state.run) held.current = { runId: state.runId, run: state.run };
  const keep = !state.run && !error && held.current !== null && held.current.run.page.id === (pageKey.page ?? held.current.run.page.id);
  const run = state.run ?? (keep ? held.current!.run : null);
  const runId = state.run ? state.runId : keep ? held.current!.runId : null;
  const stale = state.stale || (keep && run !== null);
  const streamId = pageKey.streamId;
  const nav = useOptionalPageNavigation();
  const page = run?.page ?? null;
  const specYaml = state.specYaml ?? pageKey.specYaml ?? null;
  useEffect(() => { if (specYaml) onSpecYaml?.(specYaml); }, [specYaml, onSpecYaml]);
  const [saveAs, setSaveAs] = useState<"wiki" | "page" | null>(null);
  const open = (uri: string) => {
    if (nav) nav.navigate({ id: uri });
    else onOpenPage?.({ id: uri });
  };
  // A page whose view is one component *is* that component: it says the
  // title and the actions, which core draws.
  const whole = page?.view.kind === "component";
  const [frameTitle, setFrameTitle] = useState<string | null>(null);
  const [frameActions, setFrameActions] = useState<FrameAction[]>([]);
  const frameHost = useRef<BridgeHost | null>(null);
  const componentHost: ComponentPageHost = useMemo(() => ({ onTitle: setFrameTitle, onActions: setFrameActions, hostRef: frameHost }), []);
  const title = page ? ((whole ? frameTitle : null) ?? tabLabel(page, run!.params, labelRow(run!))) : "";
  usePageTitle(title || null);

  const registry = useMemo(createNodeRegistry, []);
  useNodeFocus(registry, focus);

  // The page's own actions bind its params, pressed on the run with no
  // node; a command that asks is confirmed as a page action's is.
  const target = useMemo(() => ({ config: { id: page?.id ?? "" }, params: run?.params ?? NO_PARAMS }), [page?.id, run?.params]);
  const pageRunner = useNodeActions(runId, null);
  const pageForms = useNodeForms(runId, null);
  const actions = useVizActions(target, streamId, pageRunner);
  // One that reads the page's row (`when`, `{{row.*}}`) reads the first
  // loader's one row, as the daemon does when it runs.
  const firstLoader = page?.loaders[0]?.name;
  const pageRows = (firstLoader ? run?.loaders[firstLoader]?.result : undefined) ?? NO_ROWS;
  const offered = (page?.actions ?? []).filter((a) => offeredWholePage(a, pageRows));
  const reads = usePageReads();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    if (!runId) return;
    reads
      .text(runId)
      .then((text) => navigator.clipboard.writeText(text))
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch((e: unknown) => recordOpError({ label: "Copy", message: e instanceof Error ? e.message : String(e) }));
  };
  const pageActions: PageAction[] = [
    // What it shows, as text, as an agent would read it.
    ...(page ? [{ id: "page-copy", label: copied ? "Copied" : "Copy", title: "Copy what the page shows, as text", standard: true, icon: "copy", run: copy }] : []),
    ...(page ? [{ id: "page-definition", label: "Show definition", standard: true, run: () => open(definitionUri(page.id, run!.params, specYaml)) }] : []),
    ...(specYaml ? [
      { id: "view-save-wiki", label: "Save to wiki…", standard: true, run: () => setSaveAs("wiki") },
      { id: "view-save-page", label: "Save as Page…", standard: true, run: () => setSaveAs("page") },
    ] : []),
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
    ...(whole
      ? frameActions.map((a) => ({
          id: a.id,
          label: a.label,
          primary: a.primary,
          prominent: a.prominent,
          submenu: a.submenu,
          icon: a.icon,
          disabled: a.disabled,
          run: () => frameHost.current?.action(a.id),
        }))
      : []),
  ];
  useTabActions(pageActions);

  // What an agent asking "what am I looking at" (`get_open_page`) reads:
  // the open run and the node landed on, under the id the tab shows.
  const pageId = nav?.ref?.id ?? null;
  useEffect(() => {
    if (!pageId) return;
    const store = getPageDetailStore();
    store.publish(pageId, { runId, focus: focus ? { node: focus } : null });
    return () => store.publish(pageId, null);
  }, [pageId, runId, focus]);

  // The rows picked for its selection params, kept in the entry's history
  // state so Back and a restart still show the row's facts.
  const [picked, setPicked] = useHistoryState<Record<string, PickedRow>>("picked", NO_PICKED);
  const apply = (name: string, value: SqlCell, row?: PickedRow) => {
    if (onChange) onChange({ [name]: value });
    else void setParams({ [name]: value });
    if (page?.params.some((p) => p.name === name && p.role === "selection")) {
      const { [name]: _, ...rest } = picked;
      setPicked(row ? { ...rest, [name]: row } : rest);
    }
  };

  // A `custom` viz shares the page's view: its filters and picks, which
  // it sets as the rail does (a replacement inside gives its own).
  const viewKeys = useMemo(() => pageViewKeys(page?.params ?? []), [page?.params]);
  const hostedView = useHostedView({
    keys: viewKeys,
    scope: "page",
    write: (changes) => {
      const patch = Object.fromEntries(Object.entries(changes).map(([k, v]) => [k, v ?? null]));
      if (onChange) onChange(patch);
      else void setParams(patch);
    },
  });

  // Comments are the page's when it declares them, on its URI (its tab's
  // identity): the nav bar steps through them, its vizes anchor them, and
  // `#comment=<id>` reveals one.
  const commentTarget = page?.comments && nav?.ref ? (identityOf(nav.ref.id) ?? nav.ref.id) : null;
  const pageComments: PageComments | null = useMemo(
    () => (commentTarget && streamId ? { target: commentTarget, streamId, threadId: pageKey.threadId } : null),
    [commentTarget, streamId, pageKey.threadId],
  );
  const reveal = page?.comments ? nav?.viewParams?.comment : null;
  useEffect(() => {
    if (typeof reveal === "string" && reveal !== "") requestCommentReveal(reveal);
  }, [reveal]);
  // A page showing a resource lists what links to it and what it links to.
  const linksIn = useBacklinks({ id: renders ?? "" });
  const linksOut = usePageOutbound({ id: renders ?? "" });
  // The body, for the rail's Contents.
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  const outOfDate = useOutOfDate(page?.origin.extension ?? null, page?.origin.deployment ?? null);
  const reading = useDelayed(!run && !error, LOADING_AFTER_MS);
  const fills = run ? whole || anyFills(run) : false;
  const full = fills || (run ? wide(run) : false);

  return (
    <Page
      testId="page-host"
      kind="page"
      titleInBody={!fills}
      width={full ? "full" : "reading"}
      fill={fills}
      actions={pageActions}
      pin={pageKey.page && page && onOpenPage ? { tile: { kind: "page", page: page.id, params: run!.params, title: page.title }, onOpenPage } : undefined}
      rightRail={
        run ? (
          <>
            <Contents root={body} />
            {saveAs && specYaml ? <SaveViewForm key={saveAs} kind={saveAs} specYaml={specYaml} initialTitle={title} onClose={() => setSaveAs(null)} onSaved={open} /> : null}
            <PageRail run={run} runId={runId} onApply={apply} picked={picked} outOfDate={outOfDate} streamId={streamId} onOpenPage={onOpenPage} />
          </>
        ) : undefined
      }
      backlinks={linksIn.length > 0 ? { count: linksIn.length, body: <BacklinksList entries={linksIn} /> } : undefined}
      outbound={linksOut.length > 0 ? { count: linksOut.length, body: <BacklinksList entries={linksOut} /> } : undefined}
      commentsNav={pageComments ? <CommentNavigator target={pageComments.target} /> : undefined}
    >
      <NodeRegistryContext.Provider value={registry}>
      <PageViewContext.Provider value={{ streamId: pageKey.streamId, threadId: pageKey.threadId }}>
      <VizStreamContext.Provider value={stream}>
      <PageCommentsContext.Provider value={pageComments}>
        <div
          ref={setBody}
          data-testid="page-file"
          data-stale={stale ? "true" : undefined}
          aria-busy={stale || undefined}
          style={fills ? fillStyle : undefined}
          {...(pageComments ? contextNodeProps(pageComments.target) : {})}
        >
          {page && !fills && !(page.category === "Dashboards" && page.view.kind === "grid" && !page.origin.bundled) ? <h1 className="ox-title">{sectionTitle(run!) ?? title}</h1> : null}
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
            <VizActionRunnerContext.Provider value={pageRunner}>
              <FormSourceContext.Provider value={pageForms}>
                <ActionForm run={target} streamId={streamId} pressed={actions.form} onClose={actions.closeForm} />
              </FormSourceContext.Provider>
            </VizActionRunnerContext.Provider>
          ) : null}
          {error ? (
            <div className="ox-error" role="alert">
              {run ? "This page couldn't run again: " : "This page couldn't run: "}
              {error}
            </div>
          ) : null}
          {run && whole ? (
            <ComponentPage run={run} stream={stream} streamId={streamId} threadId={pageKey.threadId} onOpenPage={onOpenPage} host={componentHost} />
          ) : run ? (
            <HostedViewContext.Provider value={hostedView}>
              <PageTree run={run} runId={runId} streamId={streamId} onOpenPage={onOpenPage} onSetParam={apply} />
            </HostedViewContext.Provider>
          ) : reading ? (
            <Loading what="the page" />
          ) : null}
        </div>
      </PageCommentsContext.Provider>
      </VizStreamContext.Provider>
      </PageViewContext.Provider>
      </NodeRegistryContext.Provider>
    </Page>
  );
}

/** The params a page's view holds (its filters and picks): what a
 *  `custom` viz on it shares and sets. */
export function pageViewKeys(params: readonly Pick<PageParam, "name" | "role">[]): string[] {
  return params.filter((p) => (p.role ?? "resource") !== "resource").map((p) => p.name);
}

/** The title of the section a switch page shows, which titles its body
 *  (as a view document's did); null for any other page. */
function sectionTitle(run: PageRun): string | null {
  const sw = switchSections(run);
  return sw?.sections.find((s) => s.id === sw.current)?.title ?? null;
}

const NO_PARAMS: Record<string, SqlCell> = {};
const NO_PICKED: Record<string, PickedRow> = {};
const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };

/** Whether one of the run's viz nodes fills (a file, a diff: its page
 *  says): the page gives its body every pixel. */
function anyFills(run: PageRun): boolean {
  const walk = (n: Node): boolean =>
    n.kind === "viz" ? n.config.fills : n.kind === "stack" || n.kind === "grid" || n.kind === "switch" ? n.children.some(walk) : false;
  return walk(run.page.view);
}

/** Whether the page wants the full width: a grid, or a board or a
 *  dashboard anywhere in it (a page's rule); everything else reads as
 *  a document. */
function wide(run: PageRun): boolean {
  const walk = (n: Node): boolean =>
    n.kind === "grid" ||
    (n.kind === "viz" && n.config.viz === "board") ||
    ((n.kind === "stack" || n.kind === "switch") && n.children.some(walk));
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
