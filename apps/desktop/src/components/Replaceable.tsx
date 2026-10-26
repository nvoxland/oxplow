import type { CSSProperties, ReactNode } from "react";

import type { SqlCell } from "../api.js";
import { HostedViewContext, useHostedView } from "../viz/hostedView.js";
import { useReplacement } from "../viz/useReplacement.js";
import { usePageRunsStore, usePageRunWith, type PageRuns } from "../pageRuns.js";
import { PageTree } from "../pages/PageTree.js";
import { declaredParams } from "../pages/pageReads.js";
import { useExtensions } from "../extensionsStore.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * A core sub-component an extension may replace (P9.A1,
 * `.context/extensions.md` → "Replacements"): `fallback` is oxplow's own,
 * shown unless the extension of the provider it is for — `provider`, or
 * the capability's active one — replaces it; then its page renders
 * instead, given `props` (the target's contract) and, for a custom
 * component, the host page's view params its contract lists (`view`:
 * Git History's filters for the history graph) — nothing else of the
 * host's — under a badge naming the extension. A replacement that can't
 * load shows `fallback` and says why.
 */
export function Replaceable({
  target,
  props,
  derived,
  streamId,
  provider = null,
  fallback,
  onOpenPage,
  runs,
}: {
  /** One of `oxplow_domain::replaceable::REPLACEABLE` (`work_item.board`). */
  target: string;
  /** The target's props contract, by name. */
  props: Record<string, SqlCell>;
  /** Values the host's loaders hold: given to a page replacement for a
   *  param it declares and `props` leaves out. */
  derived?: Record<string, SqlCell>;
  streamId: string | null;
  /** The provider whose thing it shows (a work item's); none: the
   *  capability's active provider decides. */
  provider?: string | null;
  /** Oxplow's own component. */
  fallback: ReactNode;
  onOpenPage?(ref: TabRef, opts?: { newTab?: boolean }): void;
  /** The page runs a page replacement opens (the window's; a test's fake). */
  runs?: PageRuns;
}) {
  const r = useReplacement(target, streamId, provider);
  // A page replacement: one run of it, its params the target's props
  // (the run is shared by key, so a fresh key object each render is fine).
  const store = usePageRunsStore();
  // Run with the props its page declares: a host passes more (its own
  // params), and a page refuses a param it doesn't declare.
  const extensions = useExtensions();
  const replacementPage = r.state === "page" ? (extensions ?? []).flatMap((e) => (e.enabled ? (e.pageFiles ?? []) : [])).find((p) => p.id === r.page) : undefined;
  const pageParams = replacementPage ? declaredParams(replacementPage, { ...derived, ...props }) : props;
  const pageRun = usePageRunWith(runs ?? store, r.state === "page" ? { page: r.page, params: pageParams, streamId, threadId: null } : null);
  const view = useHostedView({ keys: r.state === "page" ? r.replacement.view : NO_KEYS, scope: target });
  if (r.state === "pending") return null;
  if (r.state === "core") return <>{fallback}</>;
  const extension = r.replacement.extension;
  const failed = (message: string) => (
    <>
      <div data-testid="replacement-fallback" style={noteStyle}>
        {`${extension}'s ${r.replacement.label} couldn't load: ${message} Showing oxplow's.`}
      </div>
      {fallback}
    </>
  );
  if (r.state === "failed") return failed(sentence(r.message));
  if (pageRun.error) return failed(sentence(pageRun.error));
  return (
    <div data-testid={`replacement-${target}`} style={replacedStyle}>
      {badge(extension, r.replacement.label)}
      {pageRun.run ? (
        // Its components share the view its target's contract lists.
        <HostedViewContext.Provider value={view}>
          <PageTree run={pageRun.run} runId={pageRun.runId} streamId={streamId} onOpenPage={onOpenPage} />
        </HostedViewContext.Provider>
      ) : null}
    </div>
  );
}

const NO_KEYS: string[] = [];

/** The note naming whose replacement shows. */
const badge = (extension: string, label: string) => (
  <span data-testid="replacement-badge" style={badgeStyle} title={`${extension} replaces oxplow's ${label}; turn it off on Settings → Integrations`}>
    replaced by {extension}
  </span>
);

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
