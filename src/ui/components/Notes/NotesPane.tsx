import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  listRecentUsage,
  listWikiNotes,
  searchWikiNotes,
  subscribeUsageEvents,
  subscribeWikiNoteEvents,
  writeWikiNoteBody,
  type Stream,
  type UsageRollup,
  type WikiNoteSearchHit,
  type WikiNoteSummary,
} from "../../api.js";
import { logUi } from "../../logger.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { formatContextMention } from "../../agent-context-ref.js";
import { ContextMenu } from "../ContextMenu.js";
import { deleteWikiNote } from "../../api.js";

type FreshnessStatus = WikiNoteSummary["freshness"];

const FRESHNESS_COLOR: Record<FreshnessStatus, string> = {
  "fresh": "var(--color-status-success, #5a9a5a)",
  "stale": "var(--color-status-warn, #c99a4a)",
  "very-stale": "var(--color-status-error, #c95a5a)",
};

const SECTION_INITIAL_LIMIT = 8;

interface Props {
  stream: Stream | null;
  selectedSlug: string | null;
  onOpenNote: (slug: string) => void;
}

export function NotesPane({ stream, selectedSlug, onOpenNote }: Props) {
  const [notes, setNotes] = useState<WikiNoteSummary[]>([]);
  const [recentUsage, setRecentUsage] = useState<UsageRollup[]>([]);
  const [query, setQuery] = useState("");
  const [searchHits, setSearchHits] = useState<WikiNoteSearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [newSlugDraft, setNewSlugDraft] = useState<string | null>(null);
  const [newSlugError, setNewSlugError] = useState<string | null>(null);
  const [showAllVisited, setShowAllVisited] = useState(false);
  const [showAllModified, setShowAllModified] = useState(false);
  const [showAllRest, setShowAllRest] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ slug: string; title: string; x: number; y: number } | null>(null);
  const newSlugInputRef = useRef<HTMLInputElement | null>(null);

  const streamId = stream?.id ?? null;

  const refreshNotes = useCallback(async () => {
    if (!streamId) {
      setNotes([]);
      return;
    }
    try {
      setNotes(await listWikiNotes(streamId));
    } catch (error) {
      logUi("error", "listWikiNotes failed", { error: String(error) });
    }
  }, [streamId]);

  const refreshUsage = useCallback(async () => {
    if (!streamId) {
      setRecentUsage([]);
      return;
    }
    try {
      setRecentUsage(await listRecentUsage({ kind: "wiki-note", streamId, limit: 32 }));
    } catch (error) {
      logUi("error", "listRecentUsage failed", { error: String(error) });
    }
  }, [streamId]);

  useEffect(() => { void refreshNotes(); }, [refreshNotes]);
  useEffect(() => { void refreshUsage(); }, [refreshUsage]);

  useEffect(() => {
    const unsub = subscribeWikiNoteEvents(() => { void refreshNotes(); });
    return unsub;
  }, [refreshNotes]);

  useEffect(() => {
    const unsub = subscribeUsageEvents(() => { void refreshUsage(); }, { kind: "wiki-note" });
    return unsub;
  }, [refreshUsage]);

  // Debounced search.
  useEffect(() => {
    if (!streamId) return;
    const trimmed = query.trim();
    if (trimmed.length === 0) {
      setSearchHits(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const handle = setTimeout(async () => {
      try {
        const hits = await searchWikiNotes(streamId, trimmed, 30);
        setSearchHits(hits);
      } catch (error) {
        logUi("error", "searchWikiNotes failed", { error: String(error) });
        setSearchHits([]);
      } finally {
        setSearching(false);
      }
    }, 150);
    return () => clearTimeout(handle);
  }, [streamId, query]);

  const beginNew = useCallback(() => {
    setNewSlugDraft("");
    setNewSlugError(null);
  }, []);

  const cancelNew = useCallback(() => {
    setNewSlugDraft(null);
    setNewSlugError(null);
  }, []);

  const submitNew = useCallback(async () => {
    if (!streamId || newSlugDraft === null) return;
    const slug = newSlugDraft.trim();
    if (!slug) {
      setNewSlugError("Slug is required.");
      return;
    }
    if (!/^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*$/.test(slug)) {
      setNewSlugError("Letters, numbers, dashes, underscores, dots only; cannot start with a dot.");
      return;
    }
    if (notes.some((n) => n.slug === slug)) {
      setNewSlugError(`A note with slug "${slug}" already exists.`);
      return;
    }
    try {
      await writeWikiNoteBody(streamId, slug, `# ${slug}\n\n`);
      setNewSlugDraft(null);
      setNewSlugError(null);
      onOpenNote(slug);
    } catch (error) {
      setNewSlugError(`Failed to create note: ${String(error)}`);
    }
  }, [streamId, notes, newSlugDraft, onOpenNote]);

  useEffect(() => {
    if (newSlugDraft !== null) newSlugInputRef.current?.focus();
  }, [newSlugDraft]);

  function openContextMenu(e: React.MouseEvent, note: { slug: string; title: string }) {
    e.preventDefault();
    setContextMenu({ slug: note.slug, title: note.title, x: e.clientX, y: e.clientY });
  }

  const contextMenuItems = contextMenu
    ? [
        { id: "notes.open", label: "Open", enabled: true, run: () => { onOpenNote(contextMenu.slug); setContextMenu(null); } },
        {
          id: "notes.add-to-agent",
          label: "Add to agent context",
          enabled: true,
          run: () => {
            insertIntoAgent(formatContextMention({ kind: "note", slug: contextMenu.slug }));
            setContextMenu(null);
          },
        },
        {
          id: "notes.delete",
          label: "Delete",
          enabled: !!streamId,
          run: async () => {
            if (!streamId) return;
            try { await deleteWikiNote(streamId, contextMenu.slug); } catch (error) {
              logUi("error", "deleteWikiNote failed", { error: String(error) });
            }
            setContextMenu(null);
          },
        },
      ]
    : [];

  const notesBySlug = useMemo(() => {
    const map = new Map<string, WikiNoteSummary>();
    for (const n of notes) map.set(n.slug, n);
    return map;
  }, [notes]);

  const visited = useMemo(() => {
    return recentUsage
      .map((u) => {
        const note = notesBySlug.get(u.key);
        if (!note) return null;
        return { note, last_at: u.last_at, count: u.count };
      })
      .filter((x): x is { note: WikiNoteSummary; last_at: string; count: number } => x !== null);
  }, [recentUsage, notesBySlug]);

  const visitedSlugs = useMemo(() => new Set(visited.map((v) => v.note.slug)), [visited]);

  const modified = useMemo(
    () => notes.filter((n) => !visitedSlugs.has(n.slug)),
    [notes, visitedSlugs],
  );

  if (!streamId) {
    return (
      <div style={{ padding: 12, color: "var(--color-text-muted, #888)" }}>
        Select a stream to view its notes.
      </div>
    );
  }

  const trimmedQuery = query.trim();
  const inSearch = trimmedQuery.length > 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "6px 8px",
        borderBottom: "1px solid var(--color-border, #333)",
        gap: 6,
      }}>
        <span style={{ fontSize: 12, opacity: 0.7 }}>Notes ({notes.length})</span>
        <button type="button" onClick={beginNew} title="New note" disabled={newSlugDraft !== null}>+ New</button>
      </div>

      <div style={{ padding: "6px 8px", borderBottom: "1px solid var(--color-border, #333)" }}>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") setQuery(""); }}
          placeholder="Search titles + bodies"
          data-testid="notes-search-input"
          style={{
            width: "100%",
            boxSizing: "border-box",
            fontSize: 12,
            padding: "4px 6px",
            background: "var(--color-bg-input, #222)",
            color: "var(--color-text, #ddd)",
            border: "1px solid var(--color-border, #333)",
          }}
        />
      </div>

      {newSlugDraft !== null && (
        <div style={{
          padding: "6px 8px",
          borderBottom: "1px solid var(--color-border, #333)",
          display: "flex",
          flexDirection: "column",
          gap: 4,
        }}>
          <input
            ref={newSlugInputRef}
            type="text"
            value={newSlugDraft}
            placeholder="note-slug"
            onChange={(e) => { setNewSlugDraft(e.target.value); setNewSlugError(null); }}
            onKeyDown={(e) => {
              if (e.key === "Enter") { e.preventDefault(); void submitNew(); }
              else if (e.key === "Escape") { e.preventDefault(); cancelNew(); }
            }}
            style={{
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 12,
              background: "var(--color-bg-input, #222)",
              color: "var(--color-text, #ddd)",
              border: "1px solid var(--color-border, #333)",
              padding: "4px 6px",
            }}
          />
          <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
            <button type="button" onClick={() => void submitNew()}>Create</button>
            <button type="button" onClick={cancelNew}>Cancel</button>
          </div>
          {newSlugError && (
            <div style={{ fontSize: 11, color: "var(--color-status-error, #c95a5a)" }}>{newSlugError}</div>
          )}
        </div>
      )}

      <div style={{ overflowY: "auto", flex: 1, minHeight: 0 }}>
        {inSearch ? (
          <SearchResults
            hits={searchHits}
            searching={searching}
            notesBySlug={notesBySlug}
            selectedSlug={selectedSlug}
            onOpenNote={onOpenNote}
            onContextMenu={(e, hit) => {
              const summary = notesBySlug.get(hit.slug);
              openContextMenu(e, summary ?? { slug: hit.slug, title: hit.title } as WikiNoteSummary);
            }}
          />
        ) : notes.length === 0 ? (
          <div style={{ padding: 12, fontSize: 12, opacity: 0.6 }}>
            No notes yet. Click "+ New" or create a file at <code>.oxplow/notes/*.md</code>.
          </div>
        ) : (
          <>
            {visited.length > 0 && (
              <Section
                title="Recently visited"
                count={visited.length}
                showAll={showAllVisited}
                onToggleShowAll={() => setShowAllVisited((v) => !v)}
                rows={(showAllVisited ? visited : visited.slice(0, SECTION_INITIAL_LIMIT)).map((v) => (
                  <NoteRow
                    key={`v-${v.note.slug}`}
                    note={v.note}
                    selected={v.note.slug === selectedSlug}
                    rightLabel={formatRelative(v.last_at)}
                    onSelect={() => onOpenNote(v.note.slug)}
                    onContextMenu={(e) => openContextMenu(e, v.note)}
                  />
                ))}
              />
            )}
            {modified.length > 0 && (
              <Section
                title={visited.length > 0 ? "Recently modified" : "Notes"}
                count={modified.length}
                showAll={showAllModified}
                onToggleShowAll={() => setShowAllModified((v) => !v)}
                rows={(showAllModified ? modified : modified.slice(0, SECTION_INITIAL_LIMIT)).map((n) => (
                  <NoteRow
                    key={`m-${n.slug}`}
                    note={n}
                    selected={n.slug === selectedSlug}
                    rightLabel={formatRelative(n.updated_at)}
                    onSelect={() => onOpenNote(n.slug)}
                    onContextMenu={(e) => openContextMenu(e, n)}
                  />
                ))}
              />
            )}
            {visited.length > 0 && modified.length > SECTION_INITIAL_LIMIT && !showAllRest && (
              <div style={{ padding: "6px 10px" }}>
                <button type="button" style={{ fontSize: 11 }} onClick={() => setShowAllRest(true)}>
                  All notes ({notes.length})
                </button>
              </div>
            )}
          </>
        )}
      </div>
      {contextMenu && (
        <ContextMenu
          items={contextMenuItems}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  );
}

function Section({
  title,
  count,
  showAll,
  onToggleShowAll,
  rows,
}: {
  title: string;
  count: number;
  showAll: boolean;
  onToggleShowAll: () => void;
  rows: React.ReactNode[];
}) {
  const showToggle = count > SECTION_INITIAL_LIMIT;
  return (
    <div>
      <div style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "6px 10px 2px",
        fontSize: 11,
        textTransform: "uppercase",
        letterSpacing: 0.5,
        opacity: 0.6,
      }}>
        <span>{title} ({count})</span>
        {showToggle && (
          <button
            type="button"
            onClick={onToggleShowAll}
            style={{ background: "none", border: "none", color: "inherit", fontSize: 11, cursor: "pointer", padding: 0 }}
          >
            {showAll ? "show less" : "show all"}
          </button>
        )}
      </div>
      {rows}
    </div>
  );
}

function SearchResults({
  hits,
  searching,
  notesBySlug,
  selectedSlug,
  onOpenNote,
  onContextMenu,
}: {
  hits: WikiNoteSearchHit[] | null;
  searching: boolean;
  notesBySlug: Map<string, WikiNoteSummary>;
  selectedSlug: string | null;
  onOpenNote: (slug: string) => void;
  onContextMenu: (e: React.MouseEvent, hit: WikiNoteSearchHit) => void;
}) {
  if (hits === null && searching) {
    return <div style={{ padding: 12, fontSize: 12, opacity: 0.6 }}>Searching…</div>;
  }
  if (hits === null) {
    return null;
  }
  if (hits.length === 0) {
    return <div style={{ padding: 12, fontSize: 12, opacity: 0.6 }}>No matches.</div>;
  }
  return (
    <>
      {hits.map((hit) => (
        <SearchRow
          key={hit.slug}
          hit={hit}
          summary={notesBySlug.get(hit.slug) ?? null}
          selected={hit.slug === selectedSlug}
          onSelect={() => onOpenNote(hit.slug)}
          onContextMenu={(e) => onContextMenu(e, hit)}
        />
      ))}
    </>
  );
}

function SearchRow({
  hit,
  summary,
  selected,
  onSelect,
  onContextMenu,
}: {
  hit: WikiNoteSearchHit;
  summary: WikiNoteSummary | null;
  selected: boolean;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const freshness = summary?.freshness ?? "fresh";
  return (
    <div
      onClick={onSelect}
      onDoubleClick={onSelect}
      onContextMenu={onContextMenu}
      draggable
      onDragStart={(e) => setContextRefDrag(e, { kind: "note", slug: hit.slug })}
      style={{
        padding: "6px 10px",
        cursor: "pointer",
        background: selected ? "var(--color-bg-selected, #2a2a2a)" : "transparent",
        borderBottom: "1px solid var(--color-border-subtle, #262626)",
        fontSize: 13,
        display: "flex",
        flexDirection: "column",
        gap: 2,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        <span style={{
          width: 8, height: 8, borderRadius: "50%",
          background: FRESHNESS_COLOR[freshness], flex: "0 0 auto",
        }} />
        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {hit.title}
        </span>
      </div>
      <div
        style={{ fontSize: 11, opacity: 0.7, paddingLeft: 14, lineHeight: 1.3 }}
        dangerouslySetInnerHTML={{ __html: highlightSnippet(hit.snippet) }}
      />
    </div>
  );
}

function NoteRow({
  note,
  selected,
  rightLabel,
  onSelect,
  onContextMenu,
}: {
  note: WikiNoteSummary;
  selected: boolean;
  rightLabel?: string;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  return (
    <div
      onClick={onSelect}
      onDoubleClick={onSelect}
      onContextMenu={onContextMenu}
      draggable
      onDragStart={(e) => setContextRefDrag(e, { kind: "note", slug: note.slug })}
      style={{
        padding: "6px 10px",
        cursor: "pointer",
        background: selected ? "var(--color-bg-selected, #2a2a2a)" : "transparent",
        borderBottom: "1px solid var(--color-border-subtle, #262626)",
        fontSize: 13,
        display: "flex",
        alignItems: "center",
        gap: 6,
      }}
      title={`${note.slug} — ${note.total_refs} referenced file${note.total_refs === 1 ? "" : "s"}\nDrag onto agent to add to context`}
    >
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: FRESHNESS_COLOR[note.freshness],
          flex: "0 0 auto",
        }}
      />
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {note.title}
      </span>
      {rightLabel && (
        <span style={{ fontSize: 11, opacity: 0.5, flex: "0 0 auto" }}>{rightLabel}</span>
      )}
    </div>
  );
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

/** Allow only the `<mark>…</mark>` tags FTS5 wraps around matches; HTML-escape everything else. */
function highlightSnippet(snippet: string): string {
  const escaped = snippet
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  return escaped
    .replace(/&lt;mark&gt;/g, '<mark style="background: var(--color-status-warn, #c99a4a); color: inherit;">')
    .replace(/&lt;\/mark&gt;/g, "</mark>");
}
