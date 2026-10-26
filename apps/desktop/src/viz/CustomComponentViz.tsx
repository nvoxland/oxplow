/// `viz: custom` (P6b.D4): the viz's component, in a sandboxed frame —
/// `allow-scripts` only, so it has an opaque origin, no network, no
/// storage and no token. It reaches the app only over the bridge
/// (`componentBridge.ts`): its declared viz nodes, its declared commands (a
/// confirmation shown here, never in the frame) and navigation. A frame
/// that never says `ready`, or navigates itself away, is torn down and the
/// viz's table shows instead — as it does with no daemon to serve the
/// bundle. Marked "custom" so the person knows it isn't oxplow's own.
import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";

import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { VizRun } from "../tauri-bridge/generated/bindings.js";
import { vizData, type HostedData } from "./componentBridge.js";
import { ComponentFrame, CustomBadge, noteStyle, useComponentBundle } from "./ComponentFrame.js";
import { useFrameForms } from "./frameForms.js";
import type { HostedView } from "./hostedView.js";

export function CustomComponentViz({
  run,
  streamId,
  fallback,
  onFailure,
  onReady,
  view = null,
  fit = false,
  data,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  run: VizRun;
  streamId: string | null;
  /** The viz's table: shown when the component can't be. */
  fallback: ReactNode;
  /** Told why when the component can't be shown — and then it shows
   *  nothing itself: the caller shows what stands in (a replacement, the
   *  core component, outside this frame). Without it, the note and
   *  `fallback`. */
  onFailure?(reason: string): void;
  /** Its component said `ready`. */
  onReady?(): void;
  /** Its host's view, where the host shares one (a replacement, a viz's
   *  own page); none in a tile or a slot. */
  view?: HostedView | null;
  /** The frame is as tall as its content says: a section of a document. */
  fit?: boolean;
  /** What it is fed (a page node's: every loader of its page); none: the
   *  viz's own rows. */
  data?: HostedData | null;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const component = run.config.custom?.component ?? null;
  const fed = useMemo(() => data ?? vizData(run), [data, run]);
  // The component itself names every call: it acts as itself wherever
  // it is shown.
  const componentKey = component ? `${run.config.extension}/${component}` : null;
  const { loadKey, version, src, error } = useComponentBundle(componentKey, base);
  // A viz's component may open forms and sign in, as a page's may.
  const frameForms = useFrameForms(componentKey, run.config.title);
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);
  const reason =
    loadKey === null
      ? "Its component can't be shown here."
      : error
        ? `Its component couldn't be loaded: ${error}.`
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
    <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", gap: 4, flex: 1, minHeight: 0 }}>
      <CustomBadge extension={run.config.extension} />
      {/* Keyed by its URL: another version (an edited bundle) mounts a new
          frame, whose first load is its own — never the old frame
          navigating away. */}
      <ComponentFrame
        key={src}
        src={src}
        version={version}
        component={componentKey ?? ""}
        data={fed}
        title={run.config.title}
        streamId={streamId}
        readyTimeoutMs={readyTimeoutMs}
        onFail={(why) => setFailed({ src, reason: why })}
        onReady={onReady}
        view={view}
        formDeps={frameForms.forms}
        fit={fit}
      />
      {frameForms.element}
    </div>
  );
}

