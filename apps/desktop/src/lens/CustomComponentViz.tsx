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

import { invokeComponentCommand, recordUsage, runComponentQuery } from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import {
  BRIDGE_PROTOCOL,
  componentBundleUrl,
  componentNavigationTarget,
  createBridgeHost,
  kitCss,
  tokensFromStyle,
} from "./componentBridge.js";

export function CustomComponentViz({
  run,
  streamId,
  onOpenPage,
  fallback,
  failure,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  run: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  /** The lens's table: shown when the component can't be. */
  fallback: ReactNode;
  /** What to show instead when the component can't be — in place of the
   *  note and `fallback` (a replacement shows the core component). */
  failure?(reason: string): ReactNode;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const component = run.lens.custom?.component ?? null;
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  if (!base || !component) return <>{failure ? failure("Its component can't be shown here.") : fallback}</>;
  const src = componentBundleUrl(base, run.lens.extension, component, streamId);
  if (failed?.src === src) {
    if (failure) return <>{failure(failed.reason)}</>;
    return (
      <div>
        <div data-testid="custom-component-fallback" style={noteStyle}>
          {failed.reason} Showing its table.
        </div>
        {fallback}
      </div>
    );
  }
  return (
    <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span data-testid="custom-component-badge" style={badgeStyle} title={`${run.lens.extension}'s own component, sandboxed`}>
        custom
      </span>
      {/* Keyed by its URL: a stream switch mounts a new frame, whose first
          load is its own — never the old frame navigating away. */}
      <ComponentFrame
        key={src}
        src={src}
        component={component}
        run={run}
        streamId={streamId}
        onOpenPage={onOpenPage}
        readyTimeoutMs={readyTimeoutMs}
        onFail={(reason) => setFailed({ src, reason })}
      />
    </div>
  );
}

/** One frame at one bundle URL and its bridge. */
function ComponentFrame({
  src,
  component,
  run,
  streamId,
  onOpenPage,
  readyTimeoutMs,
  onFail,
}: {
  src: string;
  component: string;
  run: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  readyTimeoutMs: number;
  onFail(reason: string): void;
}) {
  const [asking, setAsking] = useState<{ command: string; answer(ok: boolean): void } | null>(null);
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
        invoke: (command, input, confirmed) =>
          invokeComponentCommand(runRef.current.lens.id, command, input, streamId, confirmed),
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
        },
      },
      initial,
    );
    const tokens = tokensFromStyle(getComputedStyle(document.documentElement));
    // An opaque-origin frame can only be addressed with "*"; the port goes
    // to this frame's window alone.
    frame.postMessage(
      {
        type: "init",
        protocol: BRIDGE_PROTOCOL,
        run: initial,
        props: initial.lens.custom?.props ?? null,
        tokens,
        kitCss: kitCss(tokens),
      },
      "*",
      [channel.port2],
    );
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
