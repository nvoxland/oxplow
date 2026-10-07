/// `viz: custom` (P6b.D4): the lens's component, in a sandboxed frame —
/// `allow-scripts` only, so it has an opaque origin, no network, no
/// storage and no token. It reaches the app only over the bridge
/// (`componentBridge.ts`): its declared lenses, its declared commands (a
/// confirmation shown here, never in the frame) and navigation. A frame
/// that never says `ready`, or navigates itself away, is torn down and the
/// lens's table shows instead — as it does with no daemon to serve the
/// bundle. Marked "custom" so the person knows it isn't oxplow's own.
import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

import { invokeComponentCommand, loadComponent, recordUsage, runComponentQuery } from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import {
  componentBundleUrl,
  componentNavigationTarget,
  componentRefusal,
  createBridgeHost,
  initMessage,
  tokensFromStyle,
} from "./componentBridge.js";

export function CustomComponentViz({
  run,
  streamId,
  onOpenPage,
  fallback,
  onFailure,
  onReady,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  run: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  /** The lens's table: shown when the component can't be. */
  fallback: ReactNode;
  /** Told why when the component can't be shown — and then it shows
   *  nothing itself: the caller shows what stands in (a replacement, the
   *  core component, outside this frame). Without it, the note and
   *  `fallback`. */
  onFailure?(reason: string): void;
  /** Its component said `ready`. */
  onReady?(): void;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const component = run.lens.custom?.component ?? null;
  const lensId = run.lens.id;
  // The bundle its frame runs, loaded for the lens (from the main
  // worktree, whatever the stream): the daemon serves the frame that
  // snapshot, by its version, and the frame invokes with it.
  const loadKey = base && component ? lensId : null;
  const [loaded, setLoaded] = useState<{ key: string; version?: string; error?: string } | null>(null);
  useEffect(() => {
    if (loadKey === null) return;
    let live = true;
    loadComponent(lensId).then(
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
    // `loadKey` names the lens it loads for.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadKey]);
  const current = loaded?.key === loadKey ? loaded : null;
  const version = current?.version ?? null;
  const src = base && version ? componentBundleUrl(base, version) : null;
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  const reason =
    loadKey === null
      ? "Its component can't be shown here."
      : current?.error
        ? `Its component couldn't be loaded: ${current.error}.`
        : src !== null && failed?.src === src
          ? failed.reason
          : null;
  useEffect(() => {
    if (reason !== null) onFailure?.(reason);
    // Once per reason; `onFailure` is the caller's, not a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reason]);
  if (reason !== null && onFailure) return null;
  if (loadKey === null) return <>{fallback}</>;
  if (reason !== null) {
    return (
      <div>
        <div data-testid="custom-component-fallback" style={noteStyle}>
          {reason} Showing its table.
        </div>
        {fallback}
      </div>
    );
  }
  if (src === null || version === null || component === null) {
    return (
      <div data-testid="custom-component-loading" style={noteStyle}>
        Loading its component…
      </div>
    );
  }
  return (
    <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span data-testid="custom-component-badge" style={badgeStyle} title={`${run.lens.extension}'s own component, sandboxed`}>
        custom
      </span>
      {/* Keyed by its URL: another version (an edited bundle) mounts a new
          frame, whose first load is its own — never the old frame
          navigating away. */}
      <ComponentFrame
        key={src}
        src={src}
        version={version}
        component={component}
        run={run}
        streamId={streamId}
        onOpenPage={onOpenPage}
        readyTimeoutMs={readyTimeoutMs}
        onFail={(why) => setFailed({ src, reason: why })}
        onReady={onReady}
      />
    </div>
  );
}

/** One frame at one bundle URL and its bridge. */
function ComponentFrame({
  src,
  version,
  component,
  run,
  streamId,
  onOpenPage,
  readyTimeoutMs,
  onFail,
  onReady,
}: {
  src: string;
  /** The bundle version it was loaded at: what it invokes with. */
  version: string;
  component: string;
  run: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  readyTimeoutMs: number;
  onFail(reason: string): void;
  onReady?(): void;
}) {
  const [asking, setAsking] = useState<{ command: string; answer(ok: boolean): void } | null>(null);
  // Why its last invoke was refused: shown here, not left to the frame.
  const [refused, setRefused] = useState<string | null>(null);
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const hostRef = useRef<ReturnType<typeof createBridgeHost> | null>(null);
  const loadsRef = useRef(0);
  const runRef = useRef(run);
  runRef.current = run;

  // A re-run reaches a ready component as `update` (the host posts only a
  // result it hasn't shown).
  useEffect(() => {
    hostRef.current?.update(run);
  }, [run]);
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
        query: (asset, params) => runComponentQuery(runRef.current.lens.id, asset, params, streamId),
        invoke: async (command, input, confirmed) => {
          try {
            const out = await invokeComponentCommand(runRef.current.lens.id, command, input, confirmed, version);
            setRefused(null);
            return out;
          } catch (e) {
            const why = componentRefusal(e);
            if (why !== null) setRefused(why);
            throw e;
          }
        },
        navigate: (ref) => {
          const tab = componentNavigationTarget(ref);
          if (tab) onOpenPage?.(tab);
          return tab !== null;
        },
        confirm: (command) => new Promise<boolean>((answer) => setAsking({ command, answer })),
        onReady: () => {
          ready = true;
          clearTimeout(timer);
          void recordUsage({ kind: "custom_component", key: `${initial.lens.extension}/${component}`, streamId }).catch(() => {});
          onReady?.();
        },
      },
      initial,
    );
    const tokens = tokensFromStyle(getComputedStyle(document.documentElement));
    // An opaque-origin frame can only be addressed with "*"; the port goes
    // to this frame's window alone.
    frame.postMessage(initMessage(initial, tokens), "*", [channel.port2]);
  };

  return (
    <>
      <iframe
        ref={frameRef}
        data-testid="custom-component-frame"
        title={run.lens.title}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        src={src}
        onLoad={onLoad}
        style={frameStyle}
      />
      {refused ? (
        <div data-testid="custom-component-refused" style={noteStyle}>
          {refused}
        </div>
      ) : null}
      {asking ? (
        <CommandConfirm
          label={run.lens.title}
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

const badgeStyle: CSSProperties = {
  alignSelf: "flex-start",
  fontSize: 10,
  padding: "0 6px",
  borderRadius: 999,
  border: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
};
const frameStyle: CSSProperties = { width: "100%", minHeight: 240, border: "1px solid var(--border-subtle)", borderRadius: 4 };
const noteStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 4 };
