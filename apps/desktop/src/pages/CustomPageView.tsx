/// A declared page whose body is a custom component (`pages:` with
/// `component:`), full-page: core's chrome and nav bar (title and actions
/// drawn by core, never by the frame) around the component's sandboxed
/// frame. The frame says the page's title and actions over the bridge
/// (`setTitle`, `setActions`), asks for a declared command's form
/// (`openForm`, shown here as a lens's form action is) and is told the
/// page's params, stream and thread. Marked "custom". The frame may hand
/// the page a lens spec (`showSpec`): the page runs it and draws it under
/// the frame with the kit, live, with Save as Lens and Pin to Dashboard,
/// the frame then at its content's height (`size`). A page declaring a
/// `rail:` lens draws it in its rail, run with the page's view; a row's
/// pick writes the view (pushed, so Back un-picks), which the frame hears.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { componentForm, getLens, invokeComponentCommand, runLens, runLensSpec, type LensRun, type SqlCell } from "../api.js";
import { PinToDashboard } from "../components/Dashboard/PinToDashboard.js";
import { SaveAsLens } from "../components/SaveAsLens.js";
import { LensResultView } from "../lens/LensResultView.js";
import { childParams, lensViewChange } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { recordOpError } from "../opErrors.js";
import type { PageAction } from "../kit/actions.js";
import { ComponentFrame, CustomBadge, noteStyle, useComponentBundle, type BridgeHost } from "../lens/ComponentFrame.js";
import type { FrameAction, PageInit } from "../lens/componentBridge.js";
import { FormSlideover } from "../lens/LensResultView.js";
import { useFormSubmit } from "../lens/useFormSubmit.js";
import { useHostedView } from "../lens/hostedView.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { PagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { ExtensionPage, FormStart, Lens, LensSpec, Stream } from "../tauri-bridge/generated/bindings.js";

interface AskedForm {
  command: string;
  input: unknown;
  version: string;
  resolve(out: { result: unknown } | null): void;
  reject(e: unknown): void;
}

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
  const [frameTitle, setFrameTitle] = useState<string | null>(null);
  const [frameActions, setFrameActions] = useState<FrameAction[]>([]);
  const [asked, setAsked] = useState<AskedForm | null>(null);
  const hostRef = useRef<BridgeHost | null>(null);
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  // The spec the frame asked the page to draw, with its first run (a
  // refused one never gets here), and the frame's content height.
  const [shown, setShown] = useState<{ spec: LensSpec; run: LensRun } | null>(null);
  const [height, setHeight] = useState<number | null>(null);

  usePageTitle(frameTitle ?? declared.title);

  const actions: PageAction[] = useMemo(
    () =>
      frameActions.map((a) => ({
        id: a.id,
        label: a.label,
        primary: a.primary,
        disabled: a.disabled,
        run: () => hostRef.current?.action(a.id),
      })),
    [frameActions],
  );

  const streamId = stream?.id ?? null;
  const streamRef = useRef(streamId);
  streamRef.current = streamId;
  // Its view params and history state are its tab entry's, whole: a view
  // change replaces the entry (a component has no selection to push;
  // going somewhere is `navigate`).
  const view = useHostedView({ keys: "all", scope: null });
  const init: PageInit = useMemo(() => ({ streamId, threadId }), [streamId, threadId]);
  const pageDeps = useMemo(
    () => ({
      setTitle: setFrameTitle,
      setActions: setFrameActions,
      openForm: (command: string, input: unknown, version: string) =>
        new Promise<{ result: unknown } | null>((resolve, reject) => {
          setAsked({ command, input, version, resolve, reject });
        }),
      // A refused spec rejects with its code and field: the frame shows it.
      showSpec: async (spec: LensSpec | null) => {
        if (spec === null) {
          setShown(null);
          return null;
        }
        const run = await runLensSpec(spec, {}, streamRef.current);
        setShown({ spec, run });
        return { columns: run.result.columns, rows: run.result.rows, truncated: run.result.truncated };
      },
      size: (h: number) => setHeight(h),
    }),
    [],
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
          run={null}
          page={init}
          pageDeps={pageDeps}
          view={view}
          hostRef={hostRef}
          height={shown ? (height ?? 120) : null}
          title={frameTitle ?? declared.title}
          streamId={streamId}
          readyTimeoutMs={readyTimeoutMs}
          onFail={(why) => setFailed({ src, reason: why })}
        />
        {shown ? <ShownSpec key={JSON.stringify(shown.spec)} spec={shown.spec} first={shown.run} stream={stream} onOpenPage={onOpenPage} /> : null}
      </div>
    );
  }

  const railLens = useLensOf(declared.rail ?? null);
  const rail = declared.rail ? <RailLens lensId={declared.rail} lens={railLens.lens} loadError={railLens.error} streamId={streamId} onOpenPage={onOpenPage} /> : undefined;
  return (
    <Page testId="page-custom" kind="page" width="full" actions={actions} rightRail={rail} rightRailTitle={railLens.lens?.title} fill={rail !== undefined}>
      {body}
      {asked && componentKey ? (
        <PageForm
          componentKey={componentKey}
          title={frameTitle ?? declared.title}
          asked={asked}
          onDone={() => setAsked(null)}
        />
      ) : null}
    </Page>
  );
}

/** The spec the frame asked the page to draw: the kit draws its run,
 *  live (run again when what it read changes), with Save as Lens and Pin
 *  to Dashboard above it. */
function ShownSpec({
  spec,
  first,
  stream,
  onOpenPage,
}: {
  spec: LensSpec;
  first: LensRun;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [run, setRun] = useState(first);
  const [error, setError] = useState<string | null>(null);
  const streamId = stream?.id ?? null;
  const refresh = useCallback(() => {
    runLensSpec(spec, {}, streamId).then(
      (next) => {
        setRun(next);
        setError(null);
      },
      (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [spec, streamId]);
  useRerunOnChange(run.result.reads ?? NO_READS, refresh);
  const chart = spec.chart ?? null;
  return (
    <div data-testid="custom-page-spec" style={specStyle}>
      <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", marginBottom: 8 }}>
        <PinToDashboard
          tile={{ kind: "query", sql: spec.query ?? "", display: spec.viz ?? "table", optionsJson: JSON.stringify(chart ? { size: "wide", chart } : { size: "wide" }) }}
          testId="custom-page-pin"
          onOpenPage={onOpenPage}
        />
        <SaveAsLens query={spec.query ?? ""} viz={spec.viz ?? "table"} chart={chart} stream={stream} onOpenPage={onOpenPage} />
      </div>
      {error ? <p className="ox-error">{error}</p> : null}
      <LensResultView run={run} onOpenPage={onOpenPage} toolbar={false} streamId={streamId} />
    </div>
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
const specStyle = { padding: "12px 16px", borderTop: "1px solid var(--border-subtle)" } as const;

/** A declared command's form the frame asked for, in a slideover: its
 *  fields start from the frame's input; submitting runs it as the
 *  component (the confirm, when it asks, is the host's) and answers the
 *  frame with its result; closing it answers none. */
function PageForm({
  componentKey,
  title,
  asked,
  onDone,
}: {
  componentKey: string;
  title: string;
  asked: AskedForm;
  onDone(): void;
}) {
  const [start, setStart] = useState<FormStart | null>(null);
  const answered = useRef(false);
  const answer = (out: { result: unknown } | null) => {
    if (answered.current) return;
    answered.current = true;
    asked.resolve(out);
    onDone();
  };
  const form = useFormSubmit(
    title,
    (input, confirmed) => invokeComponentCommand(componentKey, asked.command, input, confirmed, asked.version),
    (outcome) => answer({ result: outcome.result }),
  );
  useEffect(() => {
    let live = true;
    componentForm(componentKey, asked.command, asked.input, asked.version).then(
      (s) => {
        if (live) setStart(s);
      },
      (e: unknown) => {
        if (!live) return;
        recordOpError({ label: title, message: e instanceof Error ? e.message : String(e) });
        answered.current = true;
        asked.reject(e);
        onDone();
      },
    );
    return () => {
      live = false;
    };
    // The form asked for is `asked`; it is fixed for this mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [asked]);
  return (
    <FormSlideover
      title={title}
      start={start}
      form={form}
      submitLabel="Submit"
      onClose={() => answer(null)}
      testId="custom-page-form"
    />
  );
}

