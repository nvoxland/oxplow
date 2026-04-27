import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getWorkItemSummaries,
  listRecentUsage,
  listWikiNotes,
  subscribeOxplowEvents,
  subscribeUsageEvents,
  subscribeWikiNoteEvents,
  type UsageRollup,
  type WikiNoteSummary,
} from "../../api.js";
import type { WorkItemStatus } from "../../../persistence/work-item-store.js";
import { logUi } from "../../logger.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { formatContextMention, type ContextRef } from "../../agent-context-ref.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { Kebab } from "../Kebab.js";
import type { MenuItem } from "../../menu.js";

const FRESHNESS_COLOR: Record<WikiNoteSummary["freshness"], string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

const INLINE_LIMIT = 4;
const FETCH_LIMIT = 30;

interface Props {
  streamId: string | null;
  onOpenNote: (slug: string) => void;
  onOpenFile: (path: string) => void;
  onOpenWorkItem: (itemId: string) => void;
}

interface NoteEntry { kind: "note"; slug: string; title: string; ts: string; freshness: WikiNoteSummary["freshness"]; }
interface FileEntry { kind: "file"; path: string; ts: string; }
interface ItemEntry { kind: "item"; itemId: string; title: string; status: WorkItemStatus; ts: string; }
type Entry = NoteEntry | FileEntry | ItemEntry;

const STATUS_COLOR: Partial<Record<WorkItemStatus, string>> = {
  ready: "var(--status-ready)",
  in_progress: "var(--status-running)",
  blocked: "var(--status-canceled)",
  done: "var(--status-done)",
  canceled: "var(--status-canceled)",
  archived: "var(--text-muted)",
};

/**
 * Compact "what have I been touching lately?" strip for the agent
 * tab. Two sections side-by-side:
 *
 *   - **Wiki** — most-recently-modified notes (`updated_at`), so the
 *     user can spot what the agent just wrote/edited.
 *   - **Files** — most-recently-visited editor files, from the
 *     `editor-file` usage events recorded by `App.handleOpenFile`.
 *
 * Each section keeps a few entries inline; everything else collapses
 * into a per-section "+N more" popover. Click → existing
 * onOpenNote / onOpenFile (which also re-records the visit).
 */
export function WikiActivityBar({ streamId, onOpenNote, onOpenFile, onOpenWorkItem }: Props) {
  const [notes, setNotes] = useState<NoteEntry[]>([]);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [items, setItems] = useState<ItemEntry[]>([]);
  const [openSection, setOpenSection] = useState<"notes" | "files" | "items" | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const refreshNotes = useCallback(async () => {
    if (!streamId) {
      setNotes([]);
      return;
    }
    try {
      const list = await listWikiNotes(streamId);
      const sorted = list.slice().sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      setNotes(sorted.map<NoteEntry>((n) => ({
        kind: "note", slug: n.slug, title: n.title, ts: n.updated_at, freshness: n.freshness,
      })));
    } catch (error) {
      logUi("error", "WikiActivityBar listWikiNotes failed", { error: String(error) });
    }
  }, [streamId]);

  const refreshItems = useCallback(async () => {
    if (!streamId) {
      setItems([]);
      return;
    }
    try {
      const rows: UsageRollup[] = await listRecentUsage({
        kind: "work-item", streamId, limit: FETCH_LIMIT,
      });
      if (rows.length === 0) {
        setItems([]);
        return;
      }
      const summaries = await getWorkItemSummaries(rows.map((r) => r.key));
      const byId = new Map(summaries.map((s) => [s.id, s]));
      // Hide rows that no longer exist (deleted items).
      const next = rows
        .map<ItemEntry | null>((r) => {
          const s = byId.get(r.key);
          if (!s) return null;
          return { kind: "item", itemId: r.key, title: s.title, status: s.status, ts: r.last_at };
        })
        .filter((x): x is ItemEntry => x !== null);
      setItems(next);
    } catch (error) {
      logUi("error", "WikiActivityBar items refresh failed", { error: String(error) });
    }
  }, [streamId]);

  const refreshFiles = useCallback(async () => {
    if (!streamId) {
      setFiles([]);
      return;
    }
    try {
      const rows: UsageRollup[] = await listRecentUsage({
        kind: "editor-file", streamId, limit: FETCH_LIMIT,
      });
      setFiles(rows.map<FileEntry>((r) => ({ kind: "file", path: r.key, ts: r.last_at })));
    } catch (error) {
      logUi("error", "WikiActivityBar listRecentUsage failed", { error: String(error) });
    }
  }, [streamId]);

  useEffect(() => { void refreshNotes(); }, [refreshNotes]);
  useEffect(() => { void refreshFiles(); }, [refreshFiles]);
  useEffect(() => { void refreshItems(); }, [refreshItems]);

  useEffect(() => {
    const unsub = subscribeWikiNoteEvents(() => { void refreshNotes(); });
    return unsub;
  }, [refreshNotes]);

  useEffect(() => {
    const unsub = subscribeUsageEvents(() => { void refreshFiles(); }, { kind: "editor-file" });
    return unsub;
  }, [refreshFiles]);

  useEffect(() => {
    const unsub = subscribeUsageEvents(() => { void refreshItems(); }, { kind: "work-item" });
    return unsub;
  }, [refreshItems]);

  // Refetch item titles/statuses when the underlying work items change
  // (rename, status flip, delete) so the bar stays in sync.
  useEffect(() => {
    const unsub = subscribeOxplowEvents((e) => {
      if (e.type === "work-item.changed" || e.type === "backlog.changed") {
        void refreshItems();
      }
    });
    return unsub;
  }, [refreshItems]);

  // Close popover on outside click / Escape.
  useEffect(() => {
    if (openSection === null) return;
    const onClick = (e: MouseEvent) => {
      if (!containerRef.current?.contains(e.target as Node)) setOpenSection(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenSection(null);
    };
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [openSection]);

  const noteInline = useMemo(() => notes.slice(0, INLINE_LIMIT), [notes]);
  const noteOverflow = useMemo(() => notes.slice(INLINE_LIMIT), [notes]);
  const fileInline = useMemo(() => files.slice(0, INLINE_LIMIT), [files]);
  const fileOverflow = useMemo(() => files.slice(INLINE_LIMIT), [files]);
  const itemInline = useMemo(() => items.slice(0, INLINE_LIMIT), [items]);
  const itemOverflow = useMemo(() => items.slice(INLINE_LIMIT), [items]);

  if (!streamId) return null;
  if (notes.length === 0 && files.length === 0 && items.length === 0) return null;

  const handleOpen = (entry: Entry) => {
    setOpenSection(null);
    if (entry.kind === "note") onOpenNote(entry.slug);
    else if (entry.kind === "file") onOpenFile(entry.path);
    else onOpenWorkItem(entry.itemId);
  };

  const buildEntryMenu = (entry: Entry): MenuItem[] => [
    {
      id: "activity.add-to-agent",
      label: "Add to agent context",
      enabled: true,
      run: () => {
        insertIntoAgent(formatContextMention(entryToContextRef(entry)));
      },
    },
  ];

  return (
    <div
      ref={containerRef}
      style={{
        position: "relative",
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "4px 8px",
        borderBottom: "1px solid var(--border-subtle)",
        background: "var(--surface-rail)",
        fontSize: 12,
        flex: "0 0 auto",
        overflow: "hidden",
        whiteSpace: "nowrap",
      }}
      data-testid="recent-activity-bar"
    >
      {notes.length > 0 && (
        <Section
          label="Wiki"
          inline={noteInline}
          overflow={noteOverflow}
          open={openSection === "notes"}
          onToggle={() => setOpenSection((v) => v === "notes" ? null : "notes")}
          onOpen={handleOpen}
          buildMenu={buildEntryMenu}
          testidPrefix="wiki-activity"
        />
      )}
      {notes.length > 0 && files.length > 0 && (
        <span style={{ width: 1, height: 14, background: "var(--border-subtle)", flex: "0 0 auto" }} />
      )}
      {files.length > 0 && (
        <Section
          label="Files"
          inline={fileInline}
          overflow={fileOverflow}
          open={openSection === "files"}
          onToggle={() => setOpenSection((v) => v === "files" ? null : "files")}
          onOpen={handleOpen}
          buildMenu={buildEntryMenu}
          testidPrefix="file-activity"
        />
      )}
      {items.length > 0 && (notes.length > 0 || files.length > 0) && (
        <span style={{ width: 1, height: 14, background: "var(--border-subtle)", flex: "0 0 auto" }} />
      )}
      {items.length > 0 && (
        <Section
          label="Items"
          inline={itemInline}
          overflow={itemOverflow}
          open={openSection === "items"}
          onToggle={() => setOpenSection((v) => v === "items" ? null : "items")}
          onOpen={handleOpen}
          buildMenu={buildEntryMenu}
          testidPrefix="item-activity"
        />
      )}
    </div>
  );
}

function Section({
  label,
  inline,
  overflow,
  open,
  onToggle,
  onOpen,
  buildMenu,
  testidPrefix,
}: {
  label: string;
  inline: Entry[];
  overflow: Entry[];
  open: boolean;
  onToggle: () => void;
  onOpen: (entry: Entry) => void;
  buildMenu: (entry: Entry) => MenuItem[];
  testidPrefix: string;
}) {
  return (
    <div style={{ position: "relative", display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
      <span style={{ opacity: 0.55, flex: "0 0 auto" }}>{label}:</span>
      {inline.map((entry) => (
        <Pill
          key={entryKey(entry)}
          entry={entry}
          onOpen={() => onOpen(entry)}
          menuItems={buildMenu(entry)}
          testidPrefix={testidPrefix}
        />
      ))}
      {overflow.length > 0 && (
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          data-testid={`${testidPrefix}-more`}
          style={{
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 10,
            padding: "1px 8px",
            cursor: "pointer",
            fontSize: 11,
            flex: "0 0 auto",
          }}
        >
          +{overflow.length} more
        </button>
      )}
      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            top: "100%",
            left: 0,
            zIndex: 30,
            marginTop: 2,
            minWidth: 260,
            maxWidth: 420,
            maxHeight: 360,
            overflowY: "auto",
            background: "var(--surface-elevated)",
            border: "1px solid var(--border-subtle)",
            boxShadow: "0 4px 14px rgba(0,0,0,0.35)",
            padding: 4,
          }}
        >
          {overflow.map((entry) => (
            <div
              key={entryKey(entry)}
              role="menuitem"
              draggable
              onDragStart={(e) => setContextRefDrag(e, entryToContextRef(entry))}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                width: "100%",
                padding: "5px 8px",
                fontSize: 12,
              }}
            >
              <button
                type="button"
                onClick={() => onOpen(entry)}
                title={entryFullLabel(entry)}
                style={{
                  display: "flex",
                  flex: 1,
                  alignItems: "center",
                  gap: 6,
                  minWidth: 0,
                  background: "transparent",
                  color: "var(--text-primary)",
                  border: "none",
                  cursor: "pointer",
                  fontSize: 12,
                  padding: 0,
                  textAlign: "left",
                }}
              >
                <EntryIcon entry={entry} />
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {entryLabel(entry)}
                </span>
                <span style={{ opacity: 0.55, fontSize: 11, flex: "0 0 auto" }}>
                  {formatRelative(entry.ts)}
                </span>
              </button>
              <Kebab
                items={buildMenu(entry)}
                size={14}
                testId={`${testidPrefix}-kebab-${entryKey(entry)}`}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Pill({
  entry,
  onOpen,
  menuItems,
  testidPrefix,
}: {
  entry: Entry;
  onOpen: () => void;
  menuItems: MenuItem[];
  testidPrefix: string;
}) {
  return (
    <span
      draggable
      onDragStart={(e) => setContextRefDrag(e, entryToContextRef(entry))}
      title={`${entryFullLabel(entry)} — ${formatRelative(entry.ts)}\nDrag onto agent to add to context`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 4,
        background: "var(--surface-card)",
        color: "var(--text-primary)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 10,
        padding: "1px 4px 1px 8px",
        cursor: "grab",
        fontSize: 11,
        flex: "0 1 auto",
        minWidth: 0,
        maxWidth: 240,
      }}
    >
      <button
        type="button"
        onClick={onOpen}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 5,
          background: "transparent",
          color: "inherit",
          border: "none",
          padding: 0,
          cursor: "pointer",
          fontSize: 11,
          minWidth: 0,
        }}
      >
        <EntryIcon entry={entry} />
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}>
          {entryLabel(entry)}
        </span>
        <span style={{ opacity: 0.55, flex: "0 0 auto" }}>· {formatRelative(entry.ts)}</span>
      </button>
      <Kebab
        items={menuItems}
        size={14}
        testId={`${testidPrefix}-kebab-${entryKey(entry)}`}
      />
    </span>
  );
}

export function entryToContextRef(entry: Entry): ContextRef {
  if (entry.kind === "note") return { kind: "note", slug: entry.slug };
  if (entry.kind === "file") return { kind: "file", path: entry.path };
  return { kind: "work-item", itemId: entry.itemId, title: entry.title, status: entry.status };
}

function EntryIcon({ entry }: { entry: Entry }) {
  if (entry.kind === "note") {
    return (
      <span style={{
        width: 7, height: 7, borderRadius: "50%",
        background: FRESHNESS_COLOR[entry.freshness], flex: "0 0 auto",
      }} />
    );
  }
  if (entry.kind === "item") {
    return (
      <span style={{
        width: 7, height: 7, borderRadius: 2,
        background: STATUS_COLOR[entry.status] ?? "var(--text-muted)",
        flex: "0 0 auto",
      }} />
    );
  }
  return (
    <span style={{ opacity: 0.55, fontSize: 10, flex: "0 0 auto" }}>📄</span>
  );
}

function entryKey(entry: Entry): string {
  if (entry.kind === "note") return `n:${entry.slug}`;
  if (entry.kind === "file") return `f:${entry.path}`;
  return `i:${entry.itemId}`;
}

function entryLabel(entry: Entry): string {
  if (entry.kind === "note") return entry.title;
  if (entry.kind === "item") return entry.title;
  return entry.path.split("/").pop() ?? entry.path;
}

function entryFullLabel(entry: Entry): string {
  if (entry.kind === "note") return entry.title;
  if (entry.kind === "item") return `${entry.title} [${entry.status}]`;
  return entry.path;
}

function formatRelative(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "";
  const diff = Date.now() - t;
  if (diff < 60_000) return "just now";
  const min = Math.floor(diff / 60_000);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const d = Math.floor(hr / 24);
  if (d < 7) return `${d}d ago`;
  if (d < 30) return `${Math.floor(d / 7)}w ago`;
  if (d < 365) return `${Math.floor(d / 30)}mo ago`;
  return `${Math.floor(d / 365)}y ago`;
}
