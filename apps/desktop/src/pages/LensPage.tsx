import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Page } from "../tabs/Page.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { useOptionalPageNavigation, usePageTitle, useTabActions } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import { getLens, runLens, type LensRun, type SqlCell } from "../api.js";
import { lensRef, withResourceParams } from "../tabs/pageRefs.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { pageView, shellView, withRenderedParams } from "./renderers.js";
import { rowRecord } from "../lens/lensActions.js";
import { lensViewChange, misplacedViewParams, viewOptions, viewParam, viewedParts } from "../lens/lensModel.js";
import { RailGroup } from "../kit/RailGroup.js";
import type { Lens } from "../tauri-bridge/generated/bindings.js";
import { useHostedView } from "../lens/hostedView.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import { LensResultView } from "../lens/LensResultView.js";
import { LensRail, type PickedRow } from "../lens/LensRail.js";
import type { PageAction } from "../kit/actions.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { LensStreamContext } from "../lens/LensStream.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { requestCommentReveal } from "../comment-reveal-bus.js";
import { contextNodeProps } from "../components/Comments/contextNodes.js";
import { Contents } from "../lens/Contents.js";
import { PageCommentsContext, type PageComments } from "../lens/pageComments.js";
import { identityOf } from "../uri/uri.js";
import { useOutOfDate } from "./railGroups.js";

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
  /** The page's name from the one row its lens returns (a document with a
   *  `query:`), used once the run has answered: the dashboard's own title.
   *  `title` is what it says until then. */
  titleFrom?: (row: Record<string, SqlCell>) => string;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
  /** The URI it renders, when it is a renderer's lens (run with `{ uri }`):
   *  the page is that URI, so another value of one of its params is that
   *  resource with the param changed (`withRenderedParams`). */
  renders?: string;
  /** Its text takes comments (`comments: true` on its `pages:` entry):
   *  on its URI, from this thread. */
  comments?: boolean;
  /** Its page offers Refresh (`refresh: true` on its `pages:` entry); the
   *  lens's own `refresh: true` does too. */
  refresh?: boolean;
  threadId?: string | null;
}

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `.oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Its rail is the one
 * place for its controls (`LensRail`): its filters, what a row click
 * picked, and where it comes from; Copy, Add to Context and its
 * own actions are the page's, in the nav bar. See `.context/extensions.md`.
 */
/** What a renderer's lens is run with, never a control: the URI it shows. */
const RENDERED = ["uri"];

export function LensPage({ lensId, params, title: pageTitle, titleFrom, stream, onOpenPage, renders, comments = false, refresh: pageRefresh = false, threadId = null }: LensPageProps) {
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
  // `#comment=<id>` on a page that takes comments asks them to reveal that
  // one (the Comments inbox's Go to); it is the page's, not a lens param.
  // `#section=<id>` is the page's on any lens page: the section a link
  // lands on, scrolled to once the body has drawn — never a lens param.
  const { section: landOn, ...shown } = pageView(nav?.viewParams ?? NO_PARAMS);
  const { comment: revealComment, ...withoutComment } = shown;
  const view = comments ? withoutComment : shown;
  const revealId = comments && typeof revealComment === "string" && revealComment !== "" ? revealComment : null;
  useEffect(() => {
    if (revealId) requestCommentReveal(revealId);
  }, [revealId]);
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
  const oneRow = run && run.result.rows.length === 1 ? rowRecord(run.result.columns, run.result.rows[0]!) : null;
  const named = (oneRow && titleFrom ? titleFrom(oneRow) : null) || (pageTitle ?? lens?.title ?? lensId);
  // A view document's page is named for the section in view: its heading
  // is the section's, its tab says "Page: Section" off the first one.
  const sections = lens ? viewOptions(lens) : NO_SECTIONS;
  const viewedPart = lens && run ? (viewedParts(lens, run.params)[0] ?? null) : null;
  const viewedLens = useLensOf(viewedPart?.lens ?? null);
  const viewing = viewedPart ? sections.find((o) => o.id === viewedPart.id) : undefined;
  const firstSection = lens ? viewParam(lens)?.default : null;
  const title = viewing?.label ?? named;
  usePageTitle(viewing && viewing.id !== firstSection ? `${named}: ${viewing.label}` : named);

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

  // The lens's own toolbar, handed up by its view for the nav bar.
  const [lensActions, setLensActions] = useState<PageAction[]>([]);
  // The same actions on the tab's right-click menu.
  useTabActions(lensActions);
  const actions: PageAction[] = [
    // A lens re-runs on its own when oxplow's data changes; Refresh is
    // for one that reads outside oxplow.
    ...(pageRefresh || lens?.refresh
      ? [{ id: "lens-refresh", label: "Refresh", standard: true, icon: "refresh-cw", run: () => void refresh() }]
      : []),
    ...lensActions,
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
    <LensRail lens={lens} values={run?.params ?? {}} picked={picked} outOfDate={outOfDate} onApply={setParam} onOpenPage={onOpenPage} fixed={renders ? RENDERED : undefined} streamId={streamId} viewed={viewedLens} />
  ) : undefined;
  // A view document's sections, each a link to it; the one in view marked.
  const sectionRail =
    lens?.view && sections.length > 0 ? (
      <RailGroup title={named} testId="lens-view-sections">
        <nav aria-label={named} className="ox-contents">
          {sections.map((o) => (
            <button
              key={o.id}
              type="button"
              className="ox-contents-item"
              data-testid={`lens-view-${o.id}`}
              aria-current={o.id === viewedPart?.id ? "page" : undefined}
              data-active={o.id === viewedPart?.id ? "true" : undefined}
              onClick={() => setParam(lens.view!, o.id)}
            >
              {o.label}
            </button>
          ))}
        </nav>
      </RailGroup>
    ) : null;

  // Comments are the page's when it declares them, on its URI (its tab's
  // identity): the nav bar steps through them, its vizes anchor them, and
  // the body is the context node plain-text selections resolve to.
  const commentTarget = comments && nav?.ref ? (identityOf(nav.ref.id) ?? nav.ref.id) : null;
  const pageComments: PageComments | null = useMemo(
    () => (commentTarget && streamId ? { target: commentTarget, streamId, threadId } : null),
    [commentTarget, streamId, threadId],
  );
  // The body, for the rail's Contents.
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  const rail = rightRail ? (
    <>
      <Contents root={body} />
      {sectionRail}
      {rightRail}
    </>
  ) : undefined;
  // A link's `#section=<id>`: scrolled to once the body holds it, and again
  // when the page shows something else.
  const landing = typeof landOn === "string" && landOn !== "" ? landOn : null;
  useEffect(() => {
    if (!landing || !body) return;
    let frame = requestAnimationFrame(function find() {
      const el = body.querySelector<HTMLElement>(`[data-section="${CSS.escape(landing)}"]`);
      if (el) el.scrollIntoView({ block: "start" });
      else frame = requestAnimationFrame(find);
    });
    const stop = setTimeout(() => cancelAnimationFrame(frame), 5000);
    return () => {
      cancelAnimationFrame(frame);
      clearTimeout(stop);
    };
  }, [landing, body, run]);

  // A page showing a resource lists what links to it and what it links
  // to (the `page_ref` graph), in the nav bar.
  const linksIn = useBacklinks({ id: renders ?? "" });
  const linksOut = usePageOutbound({ id: renders ?? "" });
  // A lens that fills (a file, a diff, a document ending in one: the
  // loader says) has the chrome's title, its body every pixel the page has.
  const fills = lens?.fills ?? false;
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
      rightRail={rail}
      backlinks={linksIn.length > 0 ? { count: linksIn.length, body: <BacklinksList entries={linksIn} /> } : undefined}
      outbound={linksOut.length > 0 ? { count: linksOut.length, body: <BacklinksList entries={linksOut} /> } : undefined}
      commentsNav={pageComments ? <CommentNavigator target={pageComments.target} /> : undefined}
    >
      <LensStreamContext.Provider value={stream}>
      <PageCommentsContext.Provider value={pageComments}>
      <div
        ref={setBody}
        style={lens?.viz === "dashboard" ? { padding: "var(--space-5)" } : fills ? fillStyle : undefined}
        {...(pageComments ? contextNodeProps(pageComments.target) : {})}
      >
      {fills ? null : <h1 className="ox-title">{title}</h1>}
      {lens?.description && !fills && !lens.view ? <p className="ox-intro">{lens.description}</p> : null}
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
            Ask an agent to fix it; it can check its work with
            <code> validate_extension</code>.
          </div>
        </div>
      ) : null}
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} onSetParam={setParam} toolbarTo={setLensActions} componentView={componentView} /> : error ? null : <p>Loading…</p>}
      </div>
      </PageCommentsContext.Provider>
      </LensStreamContext.Provider>
    </Page>
  );
}

const NO_PARAMS: Record<string, SqlCell> = {};
const NO_SECTIONS: { id: string; label: string }[] = [];

/** Lens `id` as loaded, once it answers; null for no id or while loading. */
function useLensOf(id: string | null): Lens | null {
  const [lens, setLens] = useState<Lens | null>(null);
  useEffect(() => {
    setLens(null);
    if (!id) return;
    let live = true;
    getLens(id)
      .then((l) => {
        if (live) setLens(l);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [id]);
  return lens;
}
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

const errorStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  margin: "12px 0",
  background: "var(--surface-card)",
};
