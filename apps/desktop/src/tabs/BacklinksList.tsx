import { EmptyState } from "../components/Prompts/EmptyState.js";
import { workItemLabel } from "../workItemRef.js";
import type { MouseEvent } from "react";
import { useMemo } from "react";
import type { MenuItem } from "../menu.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import { PageKindIcon } from "../pageKinds.js";
import { kindOf, parseRef, refOf } from "../refs/ref.js";
import { openRefInWindow } from "./openRef.js";
import { refTargetNow } from "./refOpener.js";
import { pageKindOf } from "./pageRefs.js";
import { setContextRefDrag } from "../agent-context-dnd.js";
import type { ContextRef } from "../agent-context-ref.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import type { NavSiblings } from "./PageNavigationContext.js";

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
  /** Optional pre-known source ("effort-end", "effort-start", …) for header. */
  source?: string;
  /** Optional pre-known free-form label for the header. */
  snapshotLabel?: string | null;
  /** Optional related tasks id (for the "Open task" affordance in the slideover). */
  tasksId?: number | null;
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
  /** Right-click actions for an entry (Unlink on a work item's link row).
   *  Without them — or with none for a row — right-click opens the page in
   *  a new tab. */
  entryMenu?(entry: BacklinkEntry): MenuItem[];
  /** Required when `snapshotEntries` is provided. Receives the snapshot
   *  id + a small descriptor so the host can route into a slideover. */
  onOpenSnapshot?(payload: { snapshotId: string; label?: string | null; source?: string; tasksId?: number | null }): void;
  /** Required when `commitEntries` is provided. */
  onOpenCommit?(payload: { sha: string; subject?: string }): void;
}

/**
 * Translate a ref into a `ContextRef` so a backlink entry can be
 * dragged into the agent terminal as an @-mention. Files / notes /
 * tasks have direct mappings; finding refs land as a file ref to
 * the finding's path because the agent doesn't have its own
 * `@finding:<id>` syntax. Returns null for ref kinds the agent can't
 * use as context (index pages, settings, etc).
 *
 * Pure — exported for tests.
 */
export function refToContextRef(ref: string): ContextRef | null {
  const parsed = parseRef(ref);
  if (parsed?.kind === "file") return { kind: "file", path: parsed.id };
  if (parsed?.kind === "wiki") return { kind: "wiki", slug: parsed.id };
  if (parsed?.kind === "work_item") {
    return { kind: "work_item", ref, title: workItemLabel(ref), state: "" };
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
  entryMenu,
  onOpenSnapshot,
  onOpenCommit,
}: BacklinksListProps) {
  const ctxNav = useOptionalPageNavigation();
  const openRef = ctxNav?.openRef ?? openRefInWindow;
  const { open: openMenu, menu } = useContextMenu();
  // Pre-compute the sibling list (the regular `entries` only — snapshot
  // and commit entries open slideovers and don't participate in tab-
  // level navigation). The destination page receives this as
  // `siblings`, picks up prev/next buttons keyed by the row's index.
  const siblingsList = useMemo<NavSiblings>(
    () => ({
      entries: entries.flatMap((e) => {
        const target = e.open ? refTargetNow(e.open) : null;
        return target && "tab" in target ? [{ ref: target.tab, label: e.label }] : [];
      }),
      // index is resolved at the destination via ref.id matching.
      index: 0,
    }),
    [entries],
  );
  const totalEntries = entries.length + snapshotEntries.length + commitEntries.length;
  if (totalEntries === 0) {
    return (
      <EmptyState
        compact
        testId="backlinks-list-empty"
        title="No backlinks yet"
        text="Pages, tasks and notes that link here show up as they're written."
      />
    );
  }
  return (
    <div data-testid="backlinks-list" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {snapshotEntries.map((entry) => (
        <button
          key={refOf("snapshot", String(entry.snapshotId))}
          type="button"
          data-testid={`backlinks-snapshot-${entry.snapshotId}`}
          onClick={() => onOpenSnapshot?.({
            snapshotId: entry.snapshotId,
            label: entry.snapshotLabel ?? null,
            source: entry.source,
            tasksId: entry.tasksId ?? null,
          })}
          disabled={!onOpenSnapshot}
          style={listButtonStyle}
        >
          <PageKindIcon kind="snapshot" size={14} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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
          key={refOf("commit", entry.sha)}
          type="button"
          data-testid={`backlinks-commit-${entry.sha}`}
          onClick={() => onOpenCommit?.({ sha: entry.sha, subject: entry.subject })}
          disabled={!onOpenCommit}
          style={listButtonStyle}
        >
          <PageKindIcon kind="commit" size={14} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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
      {entries.map((entry, idx) => {
        const page = entry.open;
        const items = entryMenu?.(entry) ?? [];
        const onContextMenu = (e: MouseEvent) => {
          if (items.length > 0) openMenu(e, items, entry.label);
          else if (page) { e.preventDefault(); openRef(page, { newTab: true }); }
          else e.preventDefault();
        };
        if (!page) {
          // Nothing opens: a dead link shows its raw ref, muted; a ref that
          // resolves but has no page shows its label.
          return (
            <div
              key={`${entry.target}|${entry.subtitle ?? ""}`}
              data-testid={entry.dead ? `backlinks-dead-${entry.target}` : `backlinks-entry-${entry.target}`}
              title={entry.dead ? DEAD_LINK_TIP : entry.target}
              onContextMenu={onContextMenu}
              style={{ ...rowStyle, cursor: "default", color: entry.dead ? "var(--text-secondary)" : "var(--text-primary)" }}
            >
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {entry.label}
              </span>
              {entry.subtitle ? (
                <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
              ) : null}
            </div>
          );
        }
        const ctxRef = refToContextRef(page);
        return (
        <button
          key={page}
          type="button"
          data-testid={`backlinks-entry-${page}`}
          onClick={(e: MouseEvent) => {
            // Cmd/Ctrl-click opens in a new tab; default is in-tab
            // navigation when a PageNavigation context is available
            // (outside one, the window's opener: a new tab).
            if ((e.metaKey || e.ctrlKey) || !ctxNav) {
              openRef(page, { newTab: true });
            } else {
              openRef(page, { siblings: { entries: siblingsList.entries, index: idx, title: "Backlinks" } });
            }
          }}
          onAuxClick={(e: MouseEvent) => {
            if (e.button === 1) { e.preventDefault(); openRef(page, { newTab: true }); }
          }}
          onContextMenu={onContextMenu}
          draggable={ctxRef !== null}
          onDragStart={ctxRef !== null ? (e) => setContextRefDrag(e, ctxRef) : undefined}
          style={rowStyle}
        >
          <PageKindIcon kind={pageKindOf(page) ?? kindOf(page) ?? "page"} size={14} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
          <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {entry.label}
          </span>
          {entry.subtitle ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
          ) : null}
        </button>
        );
      })}
      {menu}
    </div>
  );
}

/** Shown on a dead link: its ref names something nothing resolves. */
const DEAD_LINK_TIP = "nothing resolves this ref; its extension may be gone";

const rowStyle = {
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
  fontSize: "var(--text-xs)",
};

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
  fontSize: "var(--text-xs)",
};
