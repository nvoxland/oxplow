import { useEffect, type CSSProperties, type ReactNode } from "react";

import { recordUsage } from "../api.js";
import { useExtensions } from "../extensionsStore.js";
import { HostedViewContext, useHostedView } from "../viz/hostedView.js";

/**
 * A node another extension's page stands in for (`RunNode.replacedBy`).
 * The page run decides whose (`.context/extensions-slots.md` → "Replacements":
 * the person's choice, the only candidate, a bound target's provider) and
 * runs it; this draws that run (`children`, drawn as an embed) under a
 * badge naming the extension, its custom components sharing the host's
 * view params the target's contract lists (Git History's filters for the
 * history graph) and nothing else of the host's. One whose page couldn't
 * run shows `fallback` — oxplow's own — and says why.
 */
export function Replaced({
  target,
  extension,
  error,
  fallback,
  streamId,
  children,
}: {
  /** One of `oxplow_domain::replaceable::REPLACEABLE` (`work_item.board`). */
  target: string;
  /** The extension whose page stands in. */
  extension: string;
  /** Why its page didn't run. */
  error: string | null;
  /** Oxplow's own component. */
  fallback: ReactNode;
  streamId: string | null;
  /** Its run, drawn. */
  children: ReactNode;
}) {
  const extensions = useExtensions();
  const replacement = (extensions ?? [])
    .flatMap((e) => e.ui.replacements)
    .find((r) => r.extension === extension && r.target === target);
  const label = replacement?.label ?? target;
  const view = useHostedView({ keys: replacement?.view ?? NO_KEYS, scope: target });
  // Usage is the evidence a kind needs to be promoted: once per
  // replacement actually shown.
  const shown = error === null ? (replacement?.id ?? `${extension}/${target}`) : null;
  useEffect(() => {
    if (shown !== null) void recordUsage({ kind: "replacement", key: shown, streamId }).catch(() => {});
  }, [shown, streamId]);
  if (error !== null) {
    return (
      <>
        <div data-testid="replacement-fallback" style={noteStyle}>
          {`${extension}'s ${label} couldn't load: ${sentence(error)} Showing oxplow's.`}
        </div>
        {fallback}
      </>
    );
  }
  return (
    // Who drew it is for the curious, not a chip on the page.
    <div
      data-testid={`replacement-${target}`}
      style={replacedStyle}
      title={`${label} by ${extension}, in place of oxplow's own; turn it off on Settings → Integrations`}
    >
      {/* Its components share the view its target's contract lists. */}
      <HostedViewContext.Provider value={view}>{children}</HostedViewContext.Provider>
    </div>
  );
}

const NO_KEYS: string[] = [];

/** `message` ending as a sentence does. */
const sentence = (message: string) => (/[.!?]$/.test(message) ? message : `${message}.`);

const replacedStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, minHeight: 0, flex: 1, padding: "8px 12px", overflow: "auto" };
const noteStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", padding: "4px 12px" };
