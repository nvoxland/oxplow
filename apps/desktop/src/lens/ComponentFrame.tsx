/// One component frame and what loads it, shared by a lens's `viz: custom`
/// (`CustomComponentViz`) and a declared page whose body is a component
/// (`pages/CustomPageView`). The frame is sandboxed (`allow-scripts
/// allow-forms`) and reaches the app only over the bridge
/// (`componentBridge.ts`).
import type { CSSProperties, MutableRefObject } from "react";
import { useEffect, useRef, useState } from "react";

import { invokeComponentCommand, loadComponent, recordUsage, runComponentQuery, runComponentSql, subscribeOxplowEvents } from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { openRefInWindow } from "../tabs/openRef.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { lensDefinitionChanged } from "./lensRerun.js";
import {
  componentMayOpen,
  componentRefusal,
  createBridgeHost,
  initMessage,
  tokensFromStyle,
  componentBundleUrl,
  type FrameAction,
  type PageInit,
} from "./componentBridge.js";

export type BridgeHost = ReturnType<typeof createBridgeHost>;

/** The page-only calls a frame shown as a page may make. */
export interface PageDeps {
  setTitle(title: string): void;
  setActions(actions: FrameAction[]): void;
  openForm(command: string, input: unknown, version: string): Promise<{ result: unknown } | null>;
  /** Change the page's view (its tab entry's params; null drops one). */
  setParams(params: Record<string, string | null>): void;
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
 *  page passes `page` (and `pageDeps`), with a null `run`. */
export function ComponentFrame({
  src,
  version,
  component,
  run,
  streamId,
  title,
  page = null,
  pageDeps,
  hostRef: hostRefProp,
  readyTimeoutMs,
  onFail,
  onReady,
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
  /** Receives the bridge host (a page presses its actions through it). */
  hostRef?: MutableRefObject<BridgeHost | null>;
  readyTimeoutMs: number;
  onFail(reason: string): void;
  onReady?(): void;
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

  // A re-run reaches a ready component as `update` (the host posts only a
  // result it hasn't shown).
  useEffect(() => {
    if (run) hostRef.current?.update(run);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run]);
  // A page's params changed (Back, Forward, its own setParams echoed): the
  // frame hears them as `params`.
  const paramsKey = page ? JSON.stringify(page.params) : null;
  useEffect(() => {
    if (page) hostRef.current?.params(page.params);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paramsKey]);
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
        ...(pageDeps
          ? {
              setTitle: (t: string) => pageDepsRef.current?.setTitle(t),
              setActions: (a: FrameAction[]) => pageDepsRef.current?.setActions(a),
              openForm: (command: string, input: unknown) =>
                pageDepsRef.current ? pageDepsRef.current.openForm(command, input, version) : Promise.resolve(null),
              setParams: (params: Record<string, string | null>) => pageDepsRef.current?.setParams(params),
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
      page?.params ?? null,
    );
    const tokens = tokensFromStyle(getComputedStyle(document.documentElement));
    // An opaque-origin frame can only be addressed with "*"; the port goes
    // to this frame's window alone.
    frame.postMessage(initMessage(initial, tokens, page), "*", [channel.port2]);
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
        style={page ? pageFrameStyle : frameStyle}
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
export const noteStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 4 };
