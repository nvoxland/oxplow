import type { CSSProperties, ReactNode } from "react";

import type { SqlCell } from "../api.js";
import { LensResultView } from "../lens/LensResultView.js";
import { REPLACEABLE_LABELS, useReplacement } from "../lens/useReplacement.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * A core sub-component an extension may replace (P9.A1,
 * `.context/extensions.md` → "Replacements"): `fallback` is oxplow's own,
 * shown unless the extension of the provider it is for — `provider`, or
 * the capability's active one — replaces it; then its lens renders
 * instead, given `props` (the target's contract) and nothing of the
 * host's, under a badge naming the extension. A replacement that can't
 * load shows `fallback` and says why.
 */
export function Replaceable({
  target,
  props,
  streamId,
  provider = null,
  fallback,
  onOpenPage,
}: {
  /** One of `oxplow_domain::replaceable::REPLACEABLE` (`work_item.board`). */
  target: string;
  /** The target's props contract, by name. */
  props: Record<string, SqlCell>;
  streamId: string | null;
  /** The provider whose thing it shows (a work item's); none: the
   *  capability's active provider decides. */
  provider?: string | null;
  /** Oxplow's own component. */
  fallback: ReactNode;
  onOpenPage?(ref: TabRef): void;
}) {
  const r = useReplacement(target, props, streamId, provider);
  if (r.state === "pending") return null;
  if (r.state === "core") return <>{fallback}</>;
  const extension = r.replacement.extension;
  const failed = (message: string) => (
    <>
      <div data-testid="replacement-fallback" style={noteStyle}>
        {`${extension}'s ${REPLACEABLE_LABELS[target] ?? target} couldn't load: ${message} Showing oxplow's.`}
      </div>
      {fallback}
    </>
  );
  if (r.state === "failed") return failed(sentence(r.message));
  return (
    <div data-testid={`replacement-${target}`} style={replacedStyle}>
      <span
        data-testid="replacement-badge"
        style={badgeStyle}
        title={`${extension} replaces oxplow's ${REPLACEABLE_LABELS[target] ?? target}; turn it off on Settings → Integrations`}
      >
        replaced by {extension}
      </span>
      {/* A custom component that can't load says so here, and the whole
          replacement gives way to oxplow's own above (tsk855). */}
      <LensResultView run={r.run} streamId={streamId} onOpenPage={onOpenPage} onCustomFailure={r.fail} onCustomReady={r.shown} />
    </div>
  );
}

/** `message` ending as a sentence does. */
const sentence = (message: string) => (/[.!?]$/.test(message) ? message : `${message}.`);

const replacedStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, minHeight: 0, flex: 1, padding: "8px 12px", overflow: "auto" };
const noteStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", padding: "4px 12px" };
const badgeStyle: CSSProperties = {
  alignSelf: "flex-start",
  fontSize: 10,
  padding: "0 6px",
  borderRadius: 999,
  border: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
};
