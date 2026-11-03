import { EmptyState } from "../components/Prompts/EmptyState.js";
import type { MouseEvent } from "react";
import { useMemo } from "react";
import type { MenuItem } from "../menu.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import { kindForTabId, PageKindIcon } from "../pageKinds.js";
import { commitUri } from "../uri/uri.js";
import { openRefInWindow } from "./openRef.js";
import { refTargetNow } from "./refOpener.js";
import { addToContext } from "../contextBasket.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import type { Origin } from "./session.js";

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
  /** Optional commit-typed backlinks. Click opens the host's
   *  CommitDetailSlideover. */
  commitEntries?: CommitBacklinkEntry[];
  /** The host's right-click actions for an entry (Unlink on a work item's
   *  link row), ahead of Add to Context, which every entry that opens a
   *  page offers. */
  entryMenu?(entry: BacklinkEntry): MenuItem[];
  /** Required when `commitEntries` is provided. */
  onOpenCommit?(payload: { sha: string; subject?: string }): void;
}

/**
 * Default renderer for a Page's `backlinks` slot. Lists each entry as a
 * button that opens the referenced page. Empty state shows a hint so
 * the panel isn't ambiguous when the list is empty.
 */
export function BacklinksList({
  entries,
  commitEntries = [],
  entryMenu,
  onOpenCommit,
}: BacklinksListProps) {
  const ctxNav = useOptionalPageNavigation();
  const openRef = ctxNav?.openRef ?? openRefInWindow;
  const { open: openMenu, menu } = useContextMenu();
  // Pre-compute the origin (the regular `entries` only — commit
  // entries open slideovers and don't participate in tab-
  // level navigation). The destination page receives this as its
  // `origin` and steps through it by the row's index.
  const originList = useMemo<Origin>(
    () => ({
      items: entries.flatMap((e) => {
        const target = e.open ? refTargetNow(e.open) : null;
        return target && "tab" in target ? [{ ref: target.tab, label: e.label }] : [];
      }),
      // index is resolved at the destination via ref.id matching.
      index: 0,
    }),
    [entries],
  );
  const totalEntries = entries.length + commitEntries.length;
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
      {commitEntries.map((entry) => (
        <button
          key={commitUri(entry.sha)}
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
        const items: MenuItem[] = [
          ...(entryMenu?.(entry) ?? []),
          // The page it opens goes in the selected thread's basket, as a
          // page row's Add to Context does.
          ...(page ? [{ id: "add-to-context", label: "Add to Context", enabled: true, run: () => void addToContext({ uri: page, label: entry.label }) }] : []),
        ];
        const onContextMenu = (e: MouseEvent) => {
          if (items.length > 0) openMenu(e, items, entry.label);
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
              openRef(page, { origin: { items: originList.items, index: idx, title: "Backlinks" } });
            }
          }}
          onAuxClick={(e: MouseEvent) => {
            if (e.button === 1) { e.preventDefault(); openRef(page, { newTab: true }); }
          }}
          onContextMenu={onContextMenu}
          style={rowStyle}
        >
          <PageKindIcon kind={kindForTabId(page)} size={14} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
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
