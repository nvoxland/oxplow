/// One component frame and what loads it, shared by a lens's `viz: custom`
/// (`CustomComponentViz`) and a declared page whose body is a component
/// (`pages/CustomPageView`). The frame is sandboxed (`allow-scripts
/// allow-forms`) and reaches the app only over the bridge
/// (`componentBridge.ts`).
import type { CSSProperties, MutableRefObject } from "react";
import { useEffect, useRef, useState } from "react";

import {
  answerSignInRedirect,
  awaitSignInRedirect,
  canCatchSignInRedirect,
  invokeComponentCommand,
  listenForSignInRedirect,
  loadComponent,
  openInSystemBrowser,
  querySql,
  recordUsage,
  runComponentQuery,
  runComponentSql,
  stopSignInRedirect,
  subscribeOxplowEvents,
} from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { openRefInWindow } from "../tabs/openRef.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { LensRun, LensSpec, SignInCompletion } from "../tauri-bridge/generated/bindings.js";
import { createSignInHost } from "./hostSignIn.js";
import { lensDefinitionChanged } from "./lensRerun.js";
import {
  componentMayOpen,
  componentRefusal,
  createBridgeHost,
  initMessage,
  tokensFromStyle,
  componentBundleUrl,
  type FrameAction,
  type OwnForm,
  type PageInit,
  type ShownRows,
} from "./componentBridge.js";
import type { HostedView } from "./hostedView.js";
import { componentMemoryProblem, useRememberedStore } from "../prefs/remembered.js";

export type BridgeHost = ReturnType<typeof createBridgeHost>;

/** The page-only calls a frame shown as a page may make. */
export interface PageDeps {
  setTitle(title: string): void;
  setActions(actions: FrameAction[]): void;
  openForm(command: string, input: unknown, version: string): Promise<{ result: unknown } | null>;
  /** Show a form of the frame's own and answer the values submitted. */
  askForm(form: OwnForm): Promise<Record<string, unknown> | null>;
  /** Run and draw a lens spec under the frame (null clears it). */
  showSpec(spec: LensSpec | null): Promise<ShownRows | null>;
  /** The frame's content height, as it changes. */
  size(height: number): void;
}

/** The bundle a component's frame runs, loaded for `componentKey` (from
 *  the main worktree, whatever the stream): the daemon serves the frame
 *  that snapshot, by its version, and the frame invokes with it. Loaded
 *  again when the activated extensions change — a hot update: a new
 *  version mounts a new frame, the same one keeps the frame it has.
 *  `loadKey` is null when nothing can load (no key, or no daemon). */
export function useComponentBundle(componentKey: string | null, base: string | null) {
  const loadKey = base && componentKey ? componentKey : null;
  const [loaded, setLoaded] = useState<{ key: string; version?: string; error?: string } | null>(null);
  const [activations, setActivations] = useState(0);
  useEffect(() => {
    if (loadKey === null) return;
    return subscribeOxplowEvents((event) => {
      if (lensDefinitionChanged(event as Record<string, unknown>)) setActivations((n) => n + 1);
    });
  }, [loadKey]);
  useEffect(() => {
    if (loadKey === null) return;
    let live = true;
    loadComponent(loadKey).then(
      (version) => {
        if (live) setLoaded({ key: loadKey, version });
      },
      (e: unknown) => {
        if (live) setLoaded({ key: loadKey, error: e instanceof Error ? e.message : String(e) });
      },
    );
    return () => {
      live = false;
    };
    // `loadKey` names the component it loads for; `activations` loads it again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadKey, activations]);
  const current = loaded?.key === loadKey ? loaded : null;
  const version = current?.version ?? null;
  const src = base && version ? componentBundleUrl(base, version) : null;
  return { loadKey, version, src, error: current?.error ?? null };
}

/** The "custom" mark: a component isn't oxplow's own. */
export function CustomBadge({ extension }: { extension: string }) {
  return (
    <span data-testid="custom-component-badge" style={badgeStyle} title={`${extension}'s own component, sandboxed`}>
      custom
    </span>
  );
}

/** One frame at one bundle URL and its bridge. A lens passes its `run`; a
 *  page passes `page` (and `pageDeps`), with a null `run`. A host that
 *  shares its view (a page, a replacement, a lens's own page) passes
 *  `view` (`useHostedView`). */
export function ComponentFrame({
  src,
  version,
  component,
  run,
  streamId,
  title,
  page = null,
  pageDeps,
  view = null,
  hostRef: hostRefProp,
  readyTimeoutMs,
  onFail,
  onReady,
  height = null,
}: {
  src: string;
  /** The bundle version it was loaded at: what it invokes with. */
  version: string;
  /** `<extension>/<component id>`: what every call names. */
  component: string;
  run: LensRun | null;
  streamId: string | null;
  /** The frame's accessible title and the confirm's label. */
  title: string;
  page?: PageInit | null;
  pageDeps?: PageDeps;
  /** Its host's view, or none (a tile, a slot). */
  view?: HostedView | null;
  /** Receives the bridge host (a page presses its actions through it). */
  hostRef?: MutableRefObject<BridgeHost | null>;
  readyTimeoutMs: number;
  onFail(reason: string): void;
  onReady?(): void;
  /** A page's frame at its content's height (what it said with `size`),
   *  when the page draws more under it; none fills the page. */
  height?: number | null;
}) {
  const nav = useOptionalPageNavigation();
  const [asking, setAsking] = useState<{ command: string; answer(ok: boolean): void } | null>(null);
  // Why its last invoke was refused: shown here, not left to the frame.
  const [refused, setRefused] = useState<string | null>(null);
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const ownHostRef = useRef<BridgeHost | null>(null);
  const hostRef = hostRefProp ?? ownHostRef;
  const loadsRef = useRef(0);
  const runRef = useRef(run);
  runRef.current = run;
  const pageDepsRef = useRef(pageDeps);
  pageDepsRef.current = pageDeps;
  const viewRef = useRef(view);
  viewRef.current = view;

  // A re-run reaches a ready component as `update` (the host posts only a
  // result it hasn't shown).
  useEffect(() => {
    if (run) hostRef.current?.update(run);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run]);
  // Its host's view params or history state changed (Back, Forward, an
  // echo of its own set): the frame hears them.
  const viewKey = view ? JSON.stringify(view.viewParams) : null;
  useEffect(() => {
    if (view) hostRef.current?.viewParams(view.viewParams);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewKey]);
  const stateKey = view ? JSON.stringify(view.historyState) : null;
  useEffect(() => {
    if (view) hostRef.current?.historyState(view.historyState);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stateKey]);
  // Its page memory: the component's own, for the person, every tab of it.
  const memory = useRememberedStore();
  const memoryRef = useRef(memory);
  memoryRef.current = memory;
  const memoryScope = `component:${component}`;
  useEffect(() => () => hostRef.current?.close(), []);

  const onLoad = () => {
    loadsRef.current += 1;
    if (loadsRef.current > 1) {
      // The frame navigated itself: nothing it shows now is the bundle.
      hostRef.current?.close();
      hostRef.current = null;
      onFail("The component navigated away.");
      return;
    }
    let ready = false;
    let channel: MessageChannel | null = null;
    const timer = setTimeout(() => {
      if (!ready) {
        channel?.port1.close();
        onFail("The component didn't start.");
      }
    }, readyTimeoutMs);
    const frame = frameRef.current?.contentWindow;
    // Unreachable, it can't say `ready`; the timer reports it.
    if (!frame) return;
    channel = new MessageChannel();
    const initial = runRef.current;
    const hosted = viewRef.current;
    const sentView = hosted ? { viewParams: hosted.viewParams, historyState: hosted.historyState } : null;
    // Signing in is the host's to run (the browser, the shell's listener):
    // the frame asks and is told how it ended. Each step is a command the
    // component declares, run as it is.
    const signIns = createSignInHost({
        canCatch: canCatchSignInRedirect,
        redirectPort: async (instance, name) => {
          const out = await querySql(
            "SELECT redirect_port FROM v_provider_credential WHERE instance = ?1 AND name = ?2",
            [instance, name],
          );
          const port = out.rows[0]?.[0];
          return typeof port === "number" ? port : null;
        },
        listen: listenForSignInRedirect,
        awaitRedirect: awaitSignInRedirect,
        answer: answerSignInRedirect,
        stop: stopSignInRedirect,
        openBrowser: openInSystemBrowser,
        begin: async (instance, name, port) =>
          (await invokeComponentCommand(component, "oxplow.provider.begin_sign_in", { instance, name, redirect_port: port }, false, version))
            .result as { url: string; signIn: number },
        complete: async (instance, name, redirect) =>
          (await invokeComponentCommand(component, "oxplow.provider.complete_sign_in", { instance, name, redirect }, false, version))
            .result as SignInCompletion,
        cancel: async (instance, name, signIn) => {
          await invokeComponentCommand(component, "oxplow.provider.cancel_sign_in", { instance, name, sign_in: signIn }, false, version);
        },
    });
    hostRef.current = createBridgeHost(
      channel.port1,
      {
        query: (asset, params) => runComponentQuery(component, asset, params, streamId, version),
        invoke: async (command, input, confirmed) => {
          try {
            const out = await invokeComponentCommand(component, command, input, confirmed, version);
            setRefused(null);
            return out;
          } catch (e) {
            const why = componentRefusal(e);
            if (why !== null) setRefused(why);
            throw e;
          }
        },
        navigate: (ref, opts) => {
          if (!componentMayOpen(ref)) return false;
          (nav?.openRef ?? openRefInWindow)(ref, opts);
          return true;
        },
        querySql: (sql, params) => runComponentSql(component, sql, params, streamId, version),
        pageMemoryGet: async (key) => {
          await memoryRef.current.ready;
          return memoryRef.current.get(memoryScope, key) ?? null;
        },
        pageMemorySet: (key, value) => {
          const problem = componentMemoryProblem(memoryRef.current, memoryScope, key, value);
          if (problem) throw Object.assign(new Error(problem), { code: "INVALID" });
          memoryRef.current.set(memoryScope, key, value === null ? undefined : value);
        },
        ...(pageDeps
          ? {
              setTitle: (t: string) => pageDepsRef.current?.setTitle(t),
              setActions: (a: FrameAction[]) => pageDepsRef.current?.setActions(a),
              openForm: (command: string, input: unknown) =>
                pageDepsRef.current ? pageDepsRef.current.openForm(command, input, version) : Promise.resolve(null),
              askForm: (form: OwnForm) => pageDepsRef.current?.askForm(form) ?? Promise.resolve(null),
              signIn: (instance: string, name: string) => signIns.signIn(instance, name),
              cancelSignIn: (instance: string, name: string) => signIns.cancel(instance, name),
              onClose: () => void signIns.closeAll(),
              showSpec: (spec: LensSpec | null) => pageDepsRef.current?.showSpec(spec) ?? Promise.resolve(null),
              size: (h: number) => pageDepsRef.current?.size(h),
            }
          : {}),
        ...(hosted
          ? {
              setViewParams: (params: Record<string, string | null>) => viewRef.current?.setViewParams(params),
              setHistoryState: (patch: Record<string, unknown>) => viewRef.current?.setHistoryState(patch),
            }
          : {}),
        confirm: (command) => new Promise<boolean>((answer) => setAsking({ command, answer })),
        onReady: () => {
          ready = true;
          clearTimeout(timer);
          void recordUsage({ kind: "custom_component", key: component, streamId }).catch(() => {});
          onReady?.();
        },
      },
      initial,
      sentView,
    );
    const tokens = tokensFromStyle(getComputedStyle(document.documentElement));
    // An opaque-origin frame can only be addressed with "*"; the port goes
    // to this frame's window alone.
    frame.postMessage(initMessage(initial, tokens, page, sentView), "*", [channel.port2]);
  };

  return (
    <>
      <iframe
        ref={frameRef}
        data-testid="custom-component-frame"
        // Its accessible name, not a `title`: WebKit shows an iframe's
        // title as a tooltip over everything the component draws.
        aria-label={title}
        sandbox="allow-scripts allow-forms"
        referrerPolicy="no-referrer"
        src={src}
        onLoad={onLoad}
        style={page ? (height !== null ? { ...sizedFrameStyle, height } : pageFrameStyle) : frameStyle}
      />
      {refused ? (
        <div data-testid="custom-component-refused" style={noteStyle}>
          {refused}
        </div>
      ) : null}
      {asking ? (
        <CommandConfirm
          label={title}
          command={asking.command}
          testIdPrefix="custom-component-confirm"
          onConfirm={() => {
            asking.answer(true);
            setAsking(null);
          }}
          onCancel={() => {
            asking.answer(false);
            setAsking(null);
          }}
        />
      ) : null}
    </>
  );
}

export const badgeStyle: CSSProperties = {
  alignSelf: "flex-start",
  fontSize: 10,
  padding: "0 6px",
  borderRadius: 999,
  border: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
};
// It grows to fill a page that gives it room (a replaced history), and is
// never shorter than a card's worth.
const frameStyle: CSSProperties = { width: "100%", flex: 1, minHeight: 240, border: "1px solid var(--border-subtle)", borderRadius: 4 };
// A page's frame is the page: no card border.
const pageFrameStyle: CSSProperties = { width: "100%", flex: 1, minHeight: 0, border: "none" };
// A page's frame above what the page draws under it: its content's height.
const sizedFrameStyle: CSSProperties = { width: "100%", flex: "none", minHeight: 120, border: "none" };
export const noteStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 4 };
