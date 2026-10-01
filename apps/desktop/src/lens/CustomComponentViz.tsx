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
import { refFromTabId } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { componentBundleUrl, createBridgeHost, kitCss, tokensFromStyle } from "./componentBridge.js";

export function CustomComponentViz({
  run,
  streamId,
  onOpenPage,
  fallback,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  run: LensRun;
  streamId: string | null;
  onOpenPage?(ref: TabRef): void;
  /** The lens's table: shown when the component can't be. */
  fallback: ReactNode;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const component = run.lens.custom?.component ?? null;
  const [failed, setFailed] = useState<string | null>(null);
  const [asking, setAsking] = useState<{ command: string; answer(ok: boolean): void } | null>(null);
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const hostRef = useRef<ReturnType<typeof createBridgeHost> | null>(null);
  const loadsRef = useRef(0);
  const runRef = useRef(run);
  runRef.current = run;

  // A re-run reaches a ready component as `update`.
  useEffect(() => {
    hostRef.current?.update(run);
  }, [run]);
  useEffect(() => () => hostRef.current?.close(), []);

  if (!base || !component) return <>{fallback}</>;
  if (failed) {
    return (
      <div>
        <div data-testid="custom-component-fallback" style={noteStyle}>
          {failed} Showing its table.
        </div>
        {fallback}
      </div>
    );
  }

  const onLoad = () => {
    loadsRef.current += 1;
    if (loadsRef.current > 1) {
      // The frame navigated itself: nothing it shows now is the bundle.
      hostRef.current?.close();
      hostRef.current = null;
      setFailed("The component navigated away.");
      return;
    }
    let ready = false;
    let channel: MessageChannel | null = null;
    const timer = setTimeout(() => {
      if (!ready) {
        channel?.port1.close();
        setFailed("The component didn't start.");
      }
    }, readyTimeoutMs);
    const frame = frameRef.current?.contentWindow;
    // Unreachable, it can't say `ready`; the timer reports it.
    if (!frame) return;
    channel = new MessageChannel();
    hostRef.current = createBridgeHost(channel.port1, {
      query: (asset, params) => runComponentQuery(runRef.current.lens.id, asset, params, streamId),
      invoke: (command, input, confirmed) =>
        invokeComponentCommand(runRef.current.lens.id, command, input, streamId, confirmed),
      navigate: (ref) => {
        const tab = refFromTabId(ref);
        if (tab) onOpenPage?.(tab);
      },
      confirm: (command) => new Promise<boolean>((answer) => setAsking({ command, answer })),
      onReady: () => {
        ready = true;
        clearTimeout(timer);
        void recordUsage({ kind: "custom_component", key: `${run.lens.extension}/${component}`, streamId }).catch(() => {});
      },
    });
    const tokens = tokensFromStyle(getComputedStyle(document.documentElement));
    // An opaque-origin frame can only be addressed with "*"; the port goes
    // to this frame's window alone.
    frame.postMessage(
      { type: "init", run: runRef.current, props: run.lens.custom?.props ?? null, tokens, kitCss: kitCss(tokens) },
      "*",
      [channel.port2],
    );
  };

  return (
    <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span data-testid="custom-component-badge" style={badgeStyle} title={`${run.lens.extension}'s own component, sandboxed`}>
        custom
      </span>
      <iframe
        ref={frameRef}
        data-testid="custom-component-frame"
        title={run.lens.title}
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        src={componentBundleUrl(base, run.lens.extension, component, streamId)}
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
    </div>
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
