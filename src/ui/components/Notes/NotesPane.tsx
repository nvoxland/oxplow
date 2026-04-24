import { useCallback, useEffect, useRef, useState } from "react";
import {
  listWikiNotes,
  subscribeWikiNoteEvents,
  writeWikiNoteBody,
  type Stream,
  type WikiNoteSummary,
} from "../../api.js";
import { logUi } from "../../logger.js";

type FreshnessStatus = WikiNoteSummary["freshness"];

const FRESHNESS_COLOR: Record<FreshnessStatus, string> = {
  "fresh": "var(--color-status-success, #5a9a5a)",
  "stale": "var(--color-status-warn, #c99a4a)",
  "very-stale": "var(--color-status-error, #c95a5a)",
};

interface Props {
  stream: Stream | null;
  selectedSlug: string | null;
  onOpenNote: (slug: string) => void;
}

export function NotesPane({ stream, selectedSlug, onOpenNote }: Props) {
  const [notes, setNotes] = useState<WikiNoteSummary[]>([]);
  const [newSlugDraft, setNewSlugDraft] = useState<string | null>(null);
  const [newSlugError, setNewSlugError] = useState<string | null>(null);
  const newSlugInputRef = useRef<HTMLInputElement | null>(null);

  const streamId = stream?.id ?? null;

  const refreshList = useCallback(async () => {
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

  useEffect(() => { void refreshList(); }, [refreshList]);

  useEffect(() => {
    const unsubscribe = subscribeWikiNoteEvents(() => { void refreshList(); });
    return unsubscribe;
  }, [refreshList]);

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

  if (!streamId) {
    return (
      <div style={{ padding: 12, color: "var(--color-text-muted, #888)" }}>
        Select a stream to view its notes.
      </div>
    );
  }

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
        {notes.length === 0 ? (
          <div style={{ padding: 12, fontSize: 12, opacity: 0.6 }}>
            No notes yet. Click "+ New" or create a file at <code>.oxplow/notes/*.md</code>.
          </div>
        ) : (
          notes.map((n) => (
            <NoteRow
              key={n.slug}
              note={n}
              selected={n.slug === selectedSlug}
              onSelect={() => onOpenNote(n.slug)}
            />
          ))
        )}
      </div>
    </div>
  );
}

function NoteRow({
  note,
  selected,
  onSelect,
}: {
  note: WikiNoteSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <div
      onClick={onSelect}
      onDoubleClick={onSelect}
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
      title={`${note.slug} — ${note.total_refs} referenced file${note.total_refs === 1 ? "" : "s"}`}
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
    </div>
  );
}
