import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  listRecentUsage,
  listWikiPages,
  searchWikiPages,
  subscribeUsageEvents,
  subscribeWikiPageEvents,
  writeWikiPageBody,
  type Stream,
  type UsageRollup,
  type WikiPageSearchHit,
  type WikiPageSummary,
} from "../../api.js";
import { logUi } from "../../logger.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { formatContextMention } from "../../agent-context-ref.js";
import { ContextMenu } from "../ContextMenu.js";
import { deleteWikiPage } from "../../api.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { wikiPageRef } from "../../tabs/pageRefs.js";
import { wikiRowTooltip } from "./wikiRowLabel.js";

type FreshnessStatus = WikiPageSummary["freshness"];

const FRESHNESS_COLOR: Record<FreshnessStatus, string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

const SECTION_INITIAL_LIMIT = 8;

interface Props {
  stream: Stream | null;
  selectedSlug: string | null;
  onOpenWikiPage: (slug: string) => void;
}

export function WikiPane({ stream, selectedSlug, onOpenWikiPage }: Props) {
  const [notes, setNotes] = useState<WikiPageSummary[]>([]);
  const [recentUsage, setRecentUsage] = useState<UsageRollup[]>([]);
  const [query, setQuery] = useState("");
  const [searchHits, setSearchHits] = useState<WikiPageSearchHit[] | null>(null);
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
      setNotes(await listWikiPages(streamId));
    } catch (error) {
      logUi("error", "listWikiPages failed", { error: String(error) });
    }
  }, [streamId]);

  const refreshUsage = useCallback(async () => {
    if (!streamId) {
      setRecentUsage([]);
      return;
    }
    try {
      setRecentUsage(await listRecentUsage({ kind: "wiki", streamId, limit: 32 }));
    } catch (error) {
      logUi("error", "listRecentUsage failed", { error: String(error) });
    }
  }, [streamId]);

  useEffect(() => { void refreshNotes(); }, [refreshNotes]);
  useEffect(() => { void refreshUsage(); }, [refreshUsage]);

  useEffect(() => {
    const unsub = subscribeWikiPageEvents(() => { void refreshNotes(); });
    return unsub;
  }, [refreshNotes]);

  useEffect(() => {
    const unsub = subscribeUsageEvents(() => { void refreshUsage(); }, { kind: "wiki" });
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
        const hits = await searchWikiPages(streamId, trimmed, 30);
        setSearchHits(hits);
      } catch (error) {
        logUi("error", "searchWikiPages failed", { error: String(error) });
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
      setNewSlugError(`A wiki page with slug "${slug}" already exists.`);
      return;
    }
    try {
      await writeWikiPageBody(streamId, slug, `# ${slug}\n\n`);
      setNewSlugDraft(null);
      setNewSlugError(null);
      onOpenWikiPage(slug);
    } catch (error) {
      setNewSlugError(`Failed to create wiki page: ${String(error)}`);
    }
  }, [streamId, notes, newSlugDraft, onOpenWikiPage]);

  useEffect(() => {
    if (newSlugDraft !== null) newSlugInputRef.current?.focus();
  }, [newSlugDraft]);

  function openMenuForWikiPage(rect: DOMRect | null, note: { slug: string; title: string }) {
    setContextMenu({
      slug: note.slug,
      title: note.title,
      x: rect ? rect.right : 0,
      y: rect ? rect.bottom + 4 : 0,
    });
  }

  const contextMenuItems = contextMenu
    ? [
        { id: "notes.open", label: "Open", enabled: true, run: () => { onOpenWikiPage(contextMenu.slug); setContextMenu(null); } },
        {
          id: "notes.add-to-agent",
          label: "Add to agent context",
          enabled: true,
          run: () => {
            insertIntoAgent(formatContextMention({ kind: "wiki", slug:contextMenu.slug }));
            setContextMenu(null);
          },
        },
        {
          id: "notes.delete",
          label: "Delete",
          enabled: !!streamId,
          run: async () => {
            if (!streamId) return;
            try { await deleteWikiPage(streamId, contextMenu.slug); } catch (error) {
              logUi("error", "deleteWikiPage failed", { error: String(error) });
            }
            setContextMenu(null);
          },
        },
      ]
    : [];

  const notesBySlug = useMemo(() => {
    const map = new Map<string, WikiPageSummary>();
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
      .filter((x): x is { note: WikiPageSummary; last_at: string; count: number } => x !== null);
  }, [recentUsage, notesBySlug]);

  const visitedSlugs = useMemo(() => new Set(visited.map((v) => v.note.slug)), [visited]);

  const modified = useMemo(
    () => notes.filter((n) => !visitedSlugs.has(n.slug)),
    [notes, visitedSlugs],
  );

  if (!streamId) {
    return (
      <div style={{ padding: 12, color: "var(--text-muted)" }}>
        Select a stream to view its wiki pages.
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
        borderBottom: "1px solid var(--border-subtle)",
        gap: 6,
      }}>
        <span style={{ fontSize: "var(--text-xs)", opacity: 0.7 }}>Wiki pages ({notes.length})</span>
        <button type="button" onClick={beginNew} title="New wiki page" disabled={newSlugDraft !== null}>+ New</button>
      </div>

      <div style={{ padding: "6px 8px", borderBottom: "1px solid var(--border-subtle)" }}>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape") setQuery(""); }}
          placeholder="Search titles + bodies"
          data-testid="wiki-pages-search-input"
          style={{
            width: "100%",
            boxSizing: "border-box",
            fontSize: "var(--text-xs)",
            padding: "4px 6px",
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-subtle)",
          }}
        />
      </div>

      {newSlugDraft !== null && (
        <div style={{
          padding: "6px 8px",
          borderBottom: "1px solid var(--border-subtle)",
          display: "flex",
          flexDirection: "column",
          gap: 4,
        }}>
          <input
            ref={newSlugInputRef}
            type="text"
            value={newSlugDraft}
            placeholder="wiki-page-slug"
            onChange={(e) => { setNewSlugDraft(e.target.value); setNewSlugError(null); }}
            onKeyDown={(e) => {
              if (e.key === "Enter") { e.preventDefault(); void submitNew(); }
              else if (e.key === "Escape") { e.preventDefault(); cancelNew(); }
            }}
            style={{
              fontFamily: "var(--font-mono, monospace)",
              fontSize: "var(--text-xs)",
              background: "var(--surface-card)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-subtle)",
              padding: "4px 6px",
            }}
          />
          <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
            <button type="button" onClick={() => void submitNew()}>Create</button>
            <button type="button" onClick={cancelNew}>Cancel</button>
          </div>
          {newSlugError && (
            <div style={{ fontSize: 11, color: "var(--severity-critical)" }}>{newSlugError}</div>
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
            onOpenWikiPage={onOpenWikiPage}
            onOpenMenu={(rect, hit) => {
              const summary = notesBySlug.get(hit.slug);
              openMenuForWikiPage(rect, summary ?? { slug: hit.slug, title: hit.title } as WikiPageSummary);
            }}
          />
        ) : notes.length === 0 ? (
          <div style={{ padding: 12, fontSize: "var(--text-xs)", opacity: 0.6 }}>
            No wiki pages yet. Click "+ New" or create a file at <code>.oxplow/wiki/*.md</code>.
          </div>
        ) : (
          <>
            {visited.length > 0 && (
              <Section
                title="Recently visited"
                count={visited.length}
                showAll={showAllVisited}
                onToggleShowAll={() => setShowAllVisited((v) => !v)}
                rows={(() => {
                  const list = showAllVisited ? visited : visited.slice(0, SECTION_INITIAL_LIMIT);
                  const siblingEntries = list.map((v) => ({ ref: wikiPageRef(v.note.slug), label: v.note.title }));
                  return list.map((v, i) => (
                    <NoteRow
                      key={`v-${v.note.slug}`}
                      note={v.note}
                      selected={v.note.slug === selectedSlug}
                      rightLabel={formatRelative(v.last_at)}
                      siblings={{ entries: siblingEntries, index: i, title: "Recently visited wiki pages" }}
                      onOpenWikiPage={onOpenWikiPage}
                      onOpenMenu={(rect, note) => openMenuForWikiPage(rect, note)}
                    />
                  ));
                })()}
              />
            )}
            {modified.length > 0 && (
              <Section
                title={visited.length > 0 ? "Recently modified" : "Wiki pages"}
                count={modified.length}
                showAll={showAllModified}
                onToggleShowAll={() => setShowAllModified((v) => !v)}
                rows={(() => {
                  const list = showAllModified ? modified : modified.slice(0, SECTION_INITIAL_LIMIT);
                  const siblingEntries = list.map((n) => ({ ref: wikiPageRef(n.slug), label: n.title }));
                  const sectionTitle = visited.length > 0 ? "Recently modified wiki pages" : "Wiki pages";
                  return list.map((n, i) => (
                    <NoteRow
                      key={`m-${n.slug}`}
                      note={n}
                      selected={n.slug === selectedSlug}
                      rightLabel={formatRelative(n.updated_at)}
                      siblings={{ entries: siblingEntries, index: i, title: sectionTitle }}
                      onOpenWikiPage={onOpenWikiPage}
                      onOpenMenu={(rect, note) => openMenuForWikiPage(rect, note)}
                    />
                  ));
                })()}
              />
            )}
            {visited.length > 0 && modified.length > SECTION_INITIAL_LIMIT && !showAllRest && (
              <div style={{ padding: "6px 10px" }}>
                <button type="button" style={{ fontSize: 11 }} onClick={() => setShowAllRest(true)}>
                  All wiki pages ({notes.length})
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
  onOpenWikiPage,
  onOpenMenu,
}: {
  hits: WikiPageSearchHit[] | null;
  searching: boolean;
  notesBySlug: Map<string, WikiPageSummary>;
  selectedSlug: string | null;
  onOpenWikiPage: (slug: string) => void;
  onOpenMenu: (rect: DOMRect, hit: WikiPageSearchHit) => void;
}) {
  if (hits === null && searching) {
    return <div style={{ padding: 12, fontSize: "var(--text-xs)", opacity: 0.6 }}>Searching…</div>;
  }
  if (hits === null) {
    return null;
  }
  if (hits.length === 0) {
    return <div style={{ padding: 12, fontSize: "var(--text-xs)", opacity: 0.6 }}>No matches.</div>;
  }
  const siblingEntries = hits.map((h) => ({ ref: wikiPageRef(h.slug), label: h.title }));
  return (
    <>
      {hits.map((hit, i) => (
        <SearchRow
          key={hit.slug}
          hit={hit}
          summary={notesBySlug.get(hit.slug) ?? null}
          selected={hit.slug === selectedSlug}
          siblings={{ entries: siblingEntries, index: i, title: "Wiki search results" }}
          onOpenWikiPage={onOpenWikiPage}
          onOpenMenu={(rect) => onOpenMenu(rect, hit)}
        />
      ))}
    </>
  );
}

function SearchRow({
  hit,
  summary,
  selected,
  siblings,
  onOpenWikiPage,
  onOpenMenu,
}: {
  hit: WikiPageSearchHit;
  summary: WikiPageSummary | null;
  selected: boolean;
  siblings?: import("../../tabs/PageNavigationContext.js").NavSiblings;
  onOpenWikiPage: (slug: string) => void;
  onOpenMenu: (rect: DOMRect) => void;
}) {
  const freshness = summary?.freshness ?? "fresh";
  const { handlers } = useRouteDispatch(wikiPageRef(hit.slug), {
    onNavigate: () => onOpenWikiPage(hit.slug),
    siblings,
  });
  return (
    <div
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      onDoubleClick={handlers.onClick}
      draggable
      onDragStart={(e) => setContextRefDrag(e, { kind: "wiki", slug:hit.slug })}
      title={hit.title}
      style={{
        padding: "10px 12px",
        cursor: "pointer",
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        borderBottom: "1px solid var(--border-subtle)",
        fontSize: "var(--text-sm)",
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
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <button
          type="button"
          aria-label="More actions"
          onClick={(e) => {
            e.stopPropagation();
            onOpenMenu((e.currentTarget as HTMLButtonElement).getBoundingClientRect());
          }}
          style={{
            background: "transparent",
            border: "none",
            color: "var(--muted)",
            cursor: "pointer",
            padding: "0 4px",
            fontSize: "var(--text-base)",
            lineHeight: 1,
          }}
        >⋯</button>
      </div>
    </div>
  );
}

function NoteRow({
  note,
  selected,
  rightLabel,
  siblings,
  onOpenWikiPage,
  onOpenMenu,
}: {
  note: WikiPageSummary;
  selected: boolean;
  rightLabel?: string;
  siblings?: import("../../tabs/PageNavigationContext.js").NavSiblings;
  onOpenWikiPage: (slug: string) => void;
  onOpenMenu: (rect: DOMRect, note: WikiPageSummary) => void;
}) {
  const { handlers } = useRouteDispatch(wikiPageRef(note.slug), {
    onNavigate: () => onOpenWikiPage(note.slug),
    siblings,
  });
  return (
    <div
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      onDoubleClick={handlers.onClick}
      draggable
      onDragStart={(e) => setContextRefDrag(e, { kind: "wiki", slug:note.slug })}
      style={{
        padding: "10px 12px",
        cursor: "pointer",
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        borderBottom: "1px solid var(--border-subtle)",
        fontSize: "var(--text-sm)",
        display: "flex",
        alignItems: "center",
        gap: 6,
      }}
      title={wikiRowTooltip(note)}
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
      <button
        type="button"
        aria-label="More actions"
        onClick={(e) => {
          e.stopPropagation();
          onOpenMenu((e.currentTarget as HTMLButtonElement).getBoundingClientRect(), note);
        }}
        style={{
          background: "transparent",
          border: "none",
          color: "var(--muted)",
          cursor: "pointer",
          padding: "0 4px",
          fontSize: "var(--text-base)",
          lineHeight: 1,
          flex: "0 0 auto",
        }}
      >⋯</button>
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
    .replace(/&lt;mark&gt;/g, '<mark style="background: var(--status-waiting); color: inherit;">')
    .replace(/&lt;\/mark&gt;/g, "</mark>");
}
