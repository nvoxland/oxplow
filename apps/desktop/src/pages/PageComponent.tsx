/// A page file's custom components (`.context/pages-and-tabs.md` "The
/// desktop host", `.context/extensions.md` "Custom components"): a
/// `component` node (`{ component: { id, props } }`) is the page's
/// extension's component in a sandboxed frame, fed the page's loaders
/// (`pageData`: every loader's rows by name, the params, its props) and
/// told when they change. Inside a stack it is a section at its content's
/// height; a page whose `view:` is one component node *is* that component
/// (`ComponentPage`, Explore Data and Timeline): it fills the page, says
/// the page's title and actions (`setTitle`, `setActions`, drawn by core),
/// shares the tab's view and may hand the page an inline page to draw under
/// it (`showView`, `ShownView`), the page's `rail:` drawn in the rail as
/// any page's is.
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type MutableRefObject } from "react";

import type { Stream } from "../api.js";
import { PinToDashboard } from "../components/Dashboard/PinToDashboard.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { SaveAsLens } from "../components/SaveAsLens.js";
import { ComponentFrame, CustomBadge, noteStyle, useComponentBundle, type BridgeHost } from "../lens/ComponentFrame.js";
import { pageData, type FrameAction, type PageInit, type ShownRows } from "../lens/componentBridge.js";
import { useFrameForms } from "../lens/frameForms.js";
import { useHostedView } from "../lens/hostedView.js";
import { usePageRunsStore, usePageRunWith, type PageRuns, type RunHandle, type RunState } from "../pageRuns.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { LensChart, LensViz, Node, PageRun } from "../tauri-bridge/generated/bindings.js";
import { NodeRegistryContext, PageTree } from "./PageTree.js";

/** How long a frame may take to say `ready`. */
const READY_MS = 3000;

/** The bundle key of component node `def` of `run`'s page: its
 *  extension's component, by id. */
function componentKey(run: PageRun, def: Node): string | null {
  return def.kind === "component" ? `${run.page.extension}/${def.component}` : null;
}

/** Why a component can't show (no daemon, a bundle that failed, a frame
 *  given up on), or that it is still loading; null: it can. */
function bundleProblem(bundle: ReturnType<typeof useComponentBundle>, failed: { src: string; reason: string } | null): { testId: string; text: string } | null {
  if (bundle.loadKey === null) return { testId: "custom-component-fallback", text: "Its component can't be shown here: it needs the daemon, which serves it." };
  if (bundle.error) return { testId: "custom-component-fallback", text: `Its component couldn't be loaded: ${bundle.error}.` };
  if (bundle.src !== null && failed?.src === bundle.src) return { testId: "custom-component-fallback", text: failed.reason };
  if (bundle.src === null || bundle.version === null) return { testId: "custom-component-loading", text: "Loading its component…" };
  return null;
}

/** A component node inside a page: its frame at its content's height. */
export function ComponentNode({
  run,
  def,
  streamId,
  base = remoteBaseUrl(),
  readyTimeoutMs = READY_MS,
}: {
  run: PageRun;
  def: Node;
  streamId: string | null;
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const key = componentKey(run, def);
  const bundle = useComponentBundle(key, base);
  const title = def.title ?? run.page.title;
  const forms = useFrameForms(key, title);
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  const data = useMemo(() => pageData(run, def.kind === "component" ? def.props : null), [run, def]);
  const problem = bundleProblem(bundle, failed);
  if (problem || !key) {
    return (
      <div data-testid={problem?.testId ?? "custom-component-fallback"} style={noteStyle}>
        {problem?.text ?? "Its component can't be shown here."}
      </div>
    );
  }
  const src = bundle.src!;
  return (
    <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <CustomBadge extension={run.page.extension} />
      <ComponentFrame
        key={src}
        src={src}
        version={bundle.version!}
        component={key}
        data={data}
        title={title}
        streamId={streamId}
        readyTimeoutMs={readyTimeoutMs}
        onFail={(why) => setFailed({ src, reason: why })}
        formDeps={forms.forms}
        fit
      />
      {forms.element}
    </div>
  );
}

/** What the page host hears from a whole-page component: its title and
 *  actions (core draws them), and the bridge its actions press through. */
export interface ComponentPageHost {
  onTitle(title: string): void;
  onActions(actions: FrameAction[]): void;
  hostRef: MutableRefObject<BridgeHost | null>;
}

/** A page that is one component: the frame fills the page, the tab's
 *  whole view is its view, and what it asks to show (`showView`) is drawn
 *  under it, the frame then at its content's height. */
export function ComponentPage({
  run,
  stream,
  streamId,
  threadId,
  onOpenPage,
  host,
  base = remoteBaseUrl(),
  readyTimeoutMs = READY_MS,
}: {
  run: PageRun;
  /** The stream whole (Save as Lens keeps it there), when the host has it. */
  stream: Stream | null;
  streamId: string | null;
  threadId: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  host: ComponentPageHost;
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const def = run.page.view;
  const key = componentKey(run, def);
  const bundle = useComponentBundle(key, base);
  const forms = useFrameForms(key, run.page.title);
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  const [height, setHeight] = useState<number | null>(null);
  const store = usePageRunsStore();
  const inline = useInlineView(store, streamId, threadId);
  // Its view params and history state are its tab entry's, whole.
  const view = useHostedView({ keys: "all", scope: null });
  const init: PageInit = useMemo(() => ({ streamId, threadId }), [streamId, threadId]);
  const hostCalls = useRef(host);
  hostCalls.current = host;
  const pageDeps = useMemo(
    () => ({
      setTitle: (t: string) => hostCalls.current.onTitle(t),
      setActions: (a: FrameAction[]) => hostCalls.current.onActions(a),
      showView: inline.showView,
      size: (h: number) => setHeight(h),
    }),
    [inline.showView],
  );
  const data = useMemo(() => pageData(run, def.kind === "component" ? def.props : null), [run, def]);
  const problem = bundleProblem(bundle, failed);
  if (bundle.loadKey === null) {
    return (
      <div style={{ padding: 16 }}>
        <EmptyState title="Needs the daemon" text="This page is an extension's own component, which the daemon serves. Connect to the daemon to see it." />
      </div>
    );
  }
  if (problem || !key) {
    return (
      <div data-testid={problem?.testId ?? "custom-component-fallback"} style={{ ...noteStyle, padding: 16 }}>
        {problem?.text ?? "Its component can't be shown here."}
      </div>
    );
  }
  const src = bundle.src!;
  return (
    <div data-testid="custom-component" style={{ ...pageStyle, overflowY: inline.shown ? "auto" : undefined }}>
      <CustomBadge extension={run.page.extension} />
      {/* Keyed by its URL: another version mounts a new frame. */}
      <ComponentFrame
        key={src}
        src={src}
        version={bundle.version!}
        component={key}
        data={data}
        page={init}
        pageDeps={pageDeps}
        formDeps={forms.forms}
        view={view}
        hostRef={host.hostRef}
        height={inline.shown ? (height ?? 120) : null}
        title={run.page.title}
        streamId={streamId}
        readyTimeoutMs={readyTimeoutMs}
        onFail={(why) => setFailed({ src, reason: why })}
      />
      {inline.shown ? <ShownView key={inline.shown} specYaml={inline.shown} stream={stream} streamId={streamId} threadId={threadId} onOpenPage={onOpenPage} /> : null}
      {forms.element}
    </div>
  );
}

/** The first answer of an opened run: its run, or why it couldn't. */
function firstAnswer(handle: RunHandle): Promise<RunState> {
  return new Promise((resolve) => {
    const now = handle.get();
    if (now.run || now.error) return resolve(now);
    const off = handle.subscribe(() => {
      const s = handle.get();
      if (!s.run && !s.error) return;
      off();
      resolve(s);
    });
  });
}

/** The inline page a component asked to show, opened once its first run
 *  answers (a page or a loader that fails is refused, `INVALID`, and
 *  nothing is drawn); held open while it shows. */
export function useInlineView(store: PageRuns, streamId: string | null, threadId: string | null) {
  const [shown, setShown] = useState<string | null>(null);
  const held = useRef<RunHandle | null>(null);
  const where = useRef({ streamId, threadId });
  where.current = { streamId, threadId };
  useEffect(
    () => () => {
      held.current?.close();
      held.current = null;
    },
    [],
  );
  const showView = useCallback(
    async (spec: string | null): Promise<ShownRows | null> => {
      if (spec === null) {
        held.current?.close();
        held.current = null;
        setShown(null);
        return null;
      }
      const handle = store.open({ specYaml: spec, params: {}, ...where.current });
      const state = await firstAnswer(handle);
      if (!state.run) {
        handle.close();
        throw Object.assign(new Error(state.error ?? "The page couldn't run."), { code: "INVALID" });
      }
      const name = state.run.page.loaders[0]?.name ?? null;
      const first = name !== null ? state.run.loaders[name] : undefined;
      if (name !== null && first?.error) {
        handle.close();
        throw Object.assign(new Error(first.error), { code: "INVALID", field: `/data/${name}/sql` });
      }
      held.current?.close();
      held.current = handle;
      setShown(spec);
      return first ? { columns: first.result.columns, rows: first.result.rows, truncated: first.result.truncated } : { columns: [], rows: [], truncated: false };
    },
    [store],
  );
  return { shown, showView };
}

/** What an inline page that is one viz over one `sql` loader keeps as a
 *  lens (Save as Lens) or a tile (Pin to Dashboard); null: it isn't. */
export function inlineLensSpec(run: PageRun): { query: string; viz: LensViz; chart: LensChart | null } | null {
  const view = run.page.view;
  const first = run.page.loaders[0];
  if (view.kind !== "viz" || !first || first.loader.kind !== "sql" || view.from !== first.name) return null;
  return { query: first.loader.sql, viz: view.lens.viz, chart: view.lens.chart ?? null };
}

/** The inline page under a component page: the kit draws its run, live,
 *  with Save as Lens and Pin to Dashboard above it. */
export function ShownView({
  specYaml,
  stream,
  streamId = stream?.id ?? null,
  threadId,
  onOpenPage,
}: {
  specYaml: string;
  stream: Stream | null;
  streamId?: string | null;
  threadId: string | null;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
}) {
  const store = usePageRunsStore();
  const state = usePageRunWith(store, { specYaml, params: {}, streamId, threadId });
  const spec = state.run ? inlineLensSpec(state.run) : null;
  return (
    <div data-testid="custom-page-spec" style={specStyle}>
      {spec && onOpenPage ? (
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end", marginBottom: 8 }}>
          <PinToDashboard
            tile={{ kind: "query", sql: spec.query, display: spec.viz, optionsJson: JSON.stringify(spec.chart ? { size: "wide", chart: spec.chart } : { size: "wide" }) }}
            testId="custom-page-pin"
            onOpenPage={onOpenPage}
          />
          <SaveAsLens query={spec.query} viz={spec.viz} chart={spec.chart} stream={stream} onOpenPage={onOpenPage} />
        </div>
      ) : null}
      {state.error ? <p className="ox-error">{state.error}</p> : null}
      {state.run ? (
        // Its nodes are its own page's: nothing in the host lands on them.
        <NodeRegistryContext.Provider value={null}>
          <PageTree run={state.run} runId={state.runId} streamId={streamId} onOpenPage={onOpenPage} />
        </NodeRegistryContext.Provider>
      ) : null}
    </div>
  );
}

const pageStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0, height: "100%" };
const specStyle: CSSProperties = { padding: "12px 16px", borderTop: "1px solid var(--border-subtle)" };
