import type { MouseEvent } from "react";
import type { BacklinkEntry } from "./backlinksIndex.js";
import type { TabRef } from "./tabState.js";
import { setContextRefDrag } from "../agent-context-dnd.js";
import type { ContextRef } from "../agent-context-ref.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";

/**
 * A backlink entry that points at a snapshot. Snapshots aren't a Page
 * kind today (there's no SnapshotPage); the host supplies an
 * `onOpenSnapshot` callback that mounts `SnapshotDetailSlideover` over
 * its own page chrome instead of navigating away. The entry carries
 * just enough metadata to render the row and prefill the slideover
 * header so it doesn't flash on open.
 */
export interface SnapshotBacklinkEntry {
  kind: "snapshot";
  snapshotId: string;
  label: string;
  /** Optional pre-known source ("task-end", "task-start", …) for header. */
  source?: string;
  /** Optional pre-known free-form label for the header. */
  snapshotLabel?: string | null;
  /** Optional related work-item id (for the "Open task" affordance in the slideover). */
  workItemId?: string | null;
  subtitle?: string;
}

/**
 * Same shape for commits. Backlinks pointing at a git commit open the
 * `CommitDetailSlideover`. The host supplies `onOpenCommit` and gets
 * back a `(sha, subject)` payload to prefill the slideover header.
 */
export interface CommitBacklinkEntry {
  kind: "commit";
  sha: string;
  /** Trimmed subject for instant header rendering. */
  subject: string;
  label: string;
  subtitle?: string;
}

export interface BacklinksListProps {
  entries: BacklinkEntry[];
  /** Optional snapshot-typed backlinks. Click opens the host's
   *  SnapshotDetailSlideover. */
  snapshotEntries?: SnapshotBacklinkEntry[];
  /** Optional commit-typed backlinks. Click opens the host's
   *  CommitDetailSlideover. */
  commitEntries?: CommitBacklinkEntry[];
  onOpenPage(ref: TabRef): void;
  /** Required when `snapshotEntries` is provided. Receives the snapshot
   *  id + a small descriptor so the host can route into a slideover. */
  onOpenSnapshot?(payload: { snapshotId: string; label?: string | null; source?: string; workItemId?: string | null }): void;
  /** Required when `commitEntries` is provided. */
  onOpenCommit?(payload: { sha: string; subject?: string }): void;
}

/**
 * Translate a `TabRef` into a `ContextRef` so a backlink entry can be
 * dragged into the agent terminal as an @-mention. Files / notes /
 * work-items have direct mappings; finding refs land as a file ref to
 * the finding's path because the agent doesn't have its own
 * `@finding:<id>` syntax. Returns null for ref kinds the agent can't
 * use as context (index pages, settings, etc).
 *
 * Pure — exported for tests.
 */
export function tabRefToContextRef(ref: TabRef): ContextRef | null {
  if (ref.kind === "file") {
    const payload = ref.payload as { path?: unknown } | null;
    if (payload && typeof payload.path === "string") {
      return { kind: "file", path: payload.path };
    }
    return null;
  }
  if (ref.kind === "note") {
    const payload = ref.payload as { slug?: unknown } | null;
    if (payload && typeof payload.slug === "string") {
      return { kind: "note", slug: payload.slug };
    }
    return null;
  }
  if (ref.kind === "work-item") {
    const payload = ref.payload as { itemId?: unknown } | null;
    if (payload && typeof payload.itemId === "string") {
      // Backlinks don't carry the title/status of the target — use a
      // placeholder that the agent can resolve via the work-item id.
      // formatContextMention reads `title` / `status`; passing the id
      // as the title keeps the inserted snippet readable.
      return { kind: "work-item", itemId: payload.itemId, title: payload.itemId, status: "" };
    }
    return null;
  }
  return null;
}

/**
 * Default renderer for a Page's `backlinks` slot. Lists each entry as a
 * button that opens the referenced page. Empty state shows a hint so
 * the panel isn't ambiguous when the list is empty.
 */
export function BacklinksList({
  entries,
  snapshotEntries = [],
  commitEntries = [],
  onOpenPage,
  onOpenSnapshot,
  onOpenCommit,
}: BacklinksListProps) {
  const ctxNav = useOptionalPageNavigation();
  const totalEntries = entries.length + snapshotEntries.length + commitEntries.length;
  if (totalEntries === 0) {
    return (
      <div data-testid="backlinks-list-empty" style={{ color: "var(--text-secondary)", fontStyle: "italic" }}>
        No backlinks yet.
      </div>
    );
  }
  return (
    <div data-testid="backlinks-list" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {snapshotEntries.map((entry) => (
        <button
          key={`snapshot:${entry.snapshotId}`}
          type="button"
          data-testid={`backlinks-snapshot-${entry.snapshotId}`}
          onClick={() => onOpenSnapshot?.({
            snapshotId: entry.snapshotId,
            label: entry.snapshotLabel ?? null,
            source: entry.source,
            workItemId: entry.workItemId ?? null,
          })}
          disabled={!onOpenSnapshot}
          style={listButtonStyle}
        >
          <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {entry.label}
          </span>
          {entry.subtitle ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
          ) : (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>snapshot</span>
          )}
        </button>
      ))}
      {commitEntries.map((entry) => (
        <button
          key={`commit:${entry.sha}`}
          type="button"
          data-testid={`backlinks-commit-${entry.sha}`}
          onClick={() => onOpenCommit?.({ sha: entry.sha, subject: entry.subject })}
          disabled={!onOpenCommit}
          style={listButtonStyle}
        >
          <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {entry.label}
          </span>
          {entry.subtitle ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
          ) : (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>commit</span>
          )}
        </button>
      ))}
      {entries.map((entry) => {
        const ctxRef = tabRefToContextRef(entry.ref);
        return (
        <button
          key={entry.ref.id}
          type="button"
          data-testid={`backlinks-entry-${entry.ref.id}`}
          onClick={(e: MouseEvent) => {
            // Cmd/Ctrl-click opens in a new tab; default is in-tab
            // navigation when a PageNavigation context is available,
            // falling back to the host's onOpenPage (new tab) otherwise.
            if ((e.metaKey || e.ctrlKey) || !ctxNav) {
              onOpenPage(entry.ref);
            } else {
              ctxNav.navigate(entry.ref);
            }
          }}
          onAuxClick={(e: MouseEvent) => {
            if (e.button === 1) { e.preventDefault(); onOpenPage(entry.ref); }
          }}
          onContextMenu={(e: MouseEvent) => { e.preventDefault(); onOpenPage(entry.ref); }}
          draggable={ctxRef !== null}
          onDragStart={ctxRef !== null ? (e) => setContextRefDrag(e, ctxRef) : undefined}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "4px 8px",
            background: "transparent",
            color: "var(--text-primary)",
            border: "1px solid transparent",
            borderRadius: 4,
            cursor: "pointer",
            textAlign: "left",
            fontSize: 12,
          }}
        >
          <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {entry.label}
          </span>
          {entry.subtitle ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
          ) : null}
        </button>
        );
      })}
    </div>
  );
}

const listButtonStyle = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "4px 8px",
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid transparent",
  borderRadius: 4,
  cursor: "pointer",
  textAlign: "left" as const,
  fontSize: 12,
};
