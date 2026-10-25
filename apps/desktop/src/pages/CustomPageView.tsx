/// A declared page whose body is a custom component (`pages:` with
/// `component:`), full-page: core's chrome and nav bar (title and actions
/// drawn by core, never by the frame) around the component's sandboxed
/// frame. The frame says the page's title and actions over the bridge
/// (`setTitle`, `setActions`), asks for a declared command's form
/// (`openForm`, shown here as a lens's form action is) and is told the
/// page's params, stream and thread. Marked "custom". The frame may hand
/// the page an inline page (`showView`): the page runs it and draws it
/// under the frame with the kit, live, with Save as Lens and Pin to
/// Dashboard (`pages/PageComponent.tsx` `ShownView`), the frame then at its
/// content's height (`size`). A page declaring a
/// `rail:` lens draws it in its rail, run with the page's view; a row's
/// pick writes the view (pushed, so Back un-picks), which the frame hears.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { usePageRunsStore } from "../pageRuns.js";
import { ShownView, useInlineView } from "./PageComponent.js";

import { getLens, runLens, type LensRun, type SqlCell } from "../api.js";
import { LensResultView } from "../lens/LensResultView.js";
import { childParams, lensViewChange } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import type { PageAction } from "../kit/actions.js";
import { ComponentFrame, CustomBadge, noteStyle, useComponentBundle, type BridgeHost } from "../lens/ComponentFrame.js";
import { type FrameAction, type HostedData, type PageInit } from "../lens/componentBridge.js";
import { useFrameForms } from "../lens/frameForms.js";
import { useHostedView } from "../lens/hostedView.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { ExtensionPage, Lens, Stream } from "../tauri-bridge/generated/bindings.js";


export function CustomPageView({
  page,
  declared,
  stream,
  threadId = null,
  onOpenPage,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  page: PagePayload;
  declared: ExtensionPage;
  stream: Stream | null;
  /** The thread it is shown in, which the frame is told. */
  threadId?: string | null;
  onOpenPage(ref: TabRef): void;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const componentKey = declared.body.kind === "component" ? declared.body.component : null;
  const bundle = useComponentBundle(componentKey, base);
  const frameForms = useFrameForms(componentKey, declared.title);
  const [frameTitle, setFrameTitle] = useState<string | null>(null);
  const [frameActions, setFrameActions] = useState<FrameAction[]>([]);
  const hostRef = useRef<BridgeHost | null>(null);
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  // The inline page the frame asked the page to draw (a refused one never
  // gets here), and the frame's content height.
  const [height, setHeight] = useState<number | null>(null);

  usePageTitle(frameTitle ?? declared.title);

  const actions: PageAction[] = useMemo(
    () =>
      frameActions.map((a) => ({
        id: a.id,
        label: a.label,
        primary: a.primary,
        prominent: a.prominent,
        submenu: a.submenu,
        icon: a.icon,
        disabled: a.disabled,
        run: () => hostRef.current?.action(a.id),
      })),
    [frameActions],
  );

  const streamId = stream?.id ?? null;
  const inline = useInlineView(usePageRunsStore(), streamId, threadId);
  const shown = inline.shown;
  // Its view params and history state are its tab entry's, whole: a view
  // change replaces the entry (a component has no selection to push;
  // going somewhere is `navigate`).
  const view = useHostedView({ keys: "all", scope: null });
  const init: PageInit = useMemo(() => ({ streamId, threadId }), [streamId, threadId]);
  const pageDeps = useMemo(
    () => ({
      setTitle: setFrameTitle,
      setActions: setFrameActions,
      // A refused page rejects with its code and field: the frame shows it.
      showView: inline.showView,
      size: (h: number) => setHeight(h),
    }),
    [inline.showView],
  );

  let body;
  if (bundle.loadKey === null) {
    body = (
      <div style={{ padding: 16 }}>
        <EmptyState title="Needs the daemon" text="This page is an extension's own component, which the daemon serves. Connect to the daemon to see it." />
      </div>
    );
  } else if (bundle.error) {
    body = (
      <div data-testid="custom-component-fallback" style={{ ...noteStyle, padding: 16 }}>
        {`Its component couldn't be loaded: ${bundle.error}.`}
      </div>
    );
  } else if (bundle.src !== null && failed?.src === bundle.src) {
    body = (
      <div data-testid="custom-component-fallback" style={{ ...noteStyle, padding: 16 }}>
        {failed.reason}
      </div>
    );
  } else if (bundle.src === null || bundle.version === null || componentKey === null) {
    body = (
      <div data-testid="custom-component-loading" style={{ ...noteStyle, padding: 16 }}>
        Loading its component…
      </div>
    );
  } else {
    const src = bundle.src;
    body = (
      <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, height: "100%", overflowY: shown ? "auto" : undefined }}>
        <CustomBadge extension={declared.extension} />
        {/* Keyed by its URL: another version mounts a new frame. */}
        <ComponentFrame
          key={src}
          src={src}
          version={bundle.version}
          component={componentKey}
          data={NO_DATA}
          page={init}
          pageDeps={pageDeps}
          formDeps={frameForms.forms}
          view={view}
          hostRef={hostRef}
          height={shown ? (height ?? 120) : null}
          title={frameTitle ?? declared.title}
          streamId={streamId}
          readyTimeoutMs={readyTimeoutMs}
          onFail={(why) => setFailed({ src, reason: why })}
        />
        {shown ? <ShownView key={shown} specYaml={shown} stream={stream} threadId={threadId} onOpenPage={onOpenPage} /> : null}
      </div>
    );
  }

  const railLens = useLensOf(declared.rail ?? null);
  const rail = declared.rail ? <RailLens lensId={declared.rail} lens={railLens.lens} loadError={railLens.error} streamId={streamId} onOpenPage={onOpenPage} /> : undefined;
  return (
    <Page testId="page-custom" kind="page" width="full" actions={actions} rightRail={rail} rightRailTitle={railLens.lens?.title} fill={rail !== undefined}>
      {body}
      {frameForms.element}
    </Page>
  );
}

/** A component page's rail: its declared lens, run with the params it
 *  takes from the page's view, live. A row's pick (a column's `select`)
 *  writes the page's view — pushed when it picks, as a lens page's — and
 *  the frame hears it. */
function RailLens({
  lensId,
  lens,
  loadError,
  streamId,
  onOpenPage,
}: {
  lensId: string;
  lens: Lens | null;
  loadError: string | null;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const nav = useOptionalPageNavigation();
  const [run, setRun] = useState<LensRun | null>(null);
  const [runError, setError] = useState<string | null>(null);
  const error = loadError ?? runError;
  const view = nav?.viewParams ?? NO_VIEW;
  const params = useMemo(() => (lens ? childParams(lens, view) : null), [lens, JSON.stringify(view)]);
  const paramsKey = JSON.stringify(params);
  const refresh = useCallback(() => {
    if (!params) return;
    runLens(lensId, params, streamId).then(
      (next) => {
        setRun(next);
        setError(null);
      },
      (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
    );
    // `paramsKey` stands for `params`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lensId, paramsKey, streamId]);
  useEffect(() => refresh(), [refresh]);
  useRerunOnChange(run?.result.reads ?? NO_READS, refresh);
  const setParam = (name: string, value: SqlCell) => {
    if (!lens || !nav?.patchViewParams) return;
    const push = lensViewChange(lens, view, { [name]: value }).push;
    nav.patchViewParams({ [name]: value }, { push });
  };
  if (error) return <p className="ox-error">{error}</p>;
  if (!run) return <p className="ox-muted">Loading…</p>;
  return (
    <div data-testid="custom-page-rail">
      <LensResultView run={run} onOpenPage={onOpenPage} toolbar={false} streamId={streamId} onSetParam={setParam} />
    </div>
  );
}

const NO_VIEW: Record<string, SqlCell> = {};
/** A declared component page is fed nothing: it reads what it shows. */
const NO_DATA: HostedData = { run: null, data: {}, params: {}, props: null };

/** Lens `id` as loaded, or why it couldn't be; none for no id. */
function useLensOf(id: string | null): { lens: Lens | null; error: string | null } {
  const [out, setOut] = useState<{ lens: Lens | null; error: string | null }>({ lens: null, error: null });
  useEffect(() => {
    setOut({ lens: null, error: null });
    if (!id) return;
    let live = true;
    getLens(id).then(
      (lens) => live && setOut({ lens, error: null }),
      (e: unknown) => live && setOut({ lens: null, error: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      live = false;
    };
  }, [id]);
  return out;
}
