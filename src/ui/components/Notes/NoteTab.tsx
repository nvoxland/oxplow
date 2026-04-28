import { useCallback, useEffect, useMemo, useState } from "react";
import {
  deleteWikiNote,
  listWikiNotes,
  readWikiNoteBody,
  subscribeWikiNoteEvents,
  writeWikiNoteBody,
  type Stream,
  type WikiNoteSummary,
} from "../../api.js";
import { MarkdownView } from "./MarkdownView.js";

type FreshnessStatus = WikiNoteSummary["freshness"];

const FRESHNESS_LABEL: Record<FreshnessStatus, string> = {
  "fresh": "fresh",
  "stale": "stale",
  "very-stale": "very stale",
};

const FRESHNESS_COLOR: Record<FreshnessStatus, string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

interface Props {
  stream: Stream;
  slug: string;
  onClosed: () => void;
  onOpenNoteInNewTab: (slug: string) => void;
  onOpenFile: (path: string) => void;
}

export function NoteTab({ stream, slug: initialSlug, onClosed, onOpenNoteInNewTab, onOpenFile }: Props) {
  const [history, setHistory] = useState<string[]>([initialSlug]);
  const [historyIdx, setHistoryIdx] = useState(0);
  const currentSlug = history[historyIdx] ?? initialSlug;

  // If the parent swaps in a new initialSlug (shouldn't happen in practice —
  // tab id is stable per slug — but guard for prop-rebinds), reset history.
  useEffect(() => {
    setHistory([initialSlug]);
    setHistoryIdx(0);
  }, [initialSlug]);

  const navigate = useCallback((nextSlug: string) => {
    if (history[historyIdx] === nextSlug) return;
    const truncated = history.slice(0, historyIdx + 1);
    setHistory([...truncated, nextSlug]);
    setHistoryIdx(truncated.length);
  }, [history, historyIdx]);

  const goBack = useCallback(() => {
    setHistoryIdx((idx) => Math.max(0, idx - 1));
  }, []);

  const goForward = useCallback(() => {
    setHistoryIdx((idx) => Math.min(history.length - 1, idx + 1));
  }, [history.length]);

  const [summary, setSummary] = useState<WikiNoteSummary | null>(null);
  const [body, setBody] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const all = await listWikiNotes(stream.id);
      setSummary(all.find((n) => n.slug === currentSlug) ?? null);
    } catch {}
    try {
      const text = await readWikiNoteBody(stream.id, currentSlug);
      setBody(text);
      setNotFound(false);
      setLoadError(null);
    } catch (error) {
      const message = String(error);
      if (/note not found/i.test(message)) {
        setNotFound(true);
        setLoadError(null);
        setBody("");
      } else {
        setLoadError(message);
        setNotFound(false);
      }
    }
  }, [stream.id, currentSlug]);

  useEffect(() => {
    void refresh();
    setEditing(false);
  }, [refresh]);

  useEffect(() => subscribeWikiNoteEvents(() => { void refresh(); }), [refresh]);

  const [draftInitialized, setDraftInitialized] = useState(false);

  // When the underlying body (re)loads, seed the draft only if the user hasn't
  // started editing yet. Otherwise leave their in-progress draft alone so
  // toggling between view/edit doesn't clobber unsaved work.
  useEffect(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
  }, [body, draftInitialized]);

  // Reset draft initialization when the slug changes — new note, new draft.
  useEffect(() => {
    setDraftInitialized(false);
  }, [currentSlug]);

  const enterEdit = useCallback(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
    setEditing(true);
  }, [body, draftInitialized]);

  const enterView = useCallback(() => {
    setEditing(false);
  }, []);

  const handleRevert = useCallback(() => {
    setDraft(body);
  }, [body]);

  const handleSave = useCallback(async () => {
    try {
      await writeWikiNoteBody(stream.id, currentSlug, draft);
      setBody(draft);
    } catch (error) {
      window.alert(`Failed to save note: ${String(error)}`);
    }
  }, [stream.id, currentSlug, draft]);

  const handleCreate = useCallback(async () => {
    const seed = `# ${currentSlug}\n\n`;
    try {
      await writeWikiNoteBody(stream.id, currentSlug, seed);
      setNotFound(false);
      setBody(seed);
      setDraft(seed);
      setDraftInitialized(true);
      setEditing(true);
    } catch (error) {
      window.alert(`Failed to create note: ${String(error)}`);
    }
  }, [stream.id, currentSlug]);

  const handleDelete = useCallback(async () => {
    if (!window.confirm(`Delete note "${currentSlug}"? The file will be removed.`)) return;
    try {
      await deleteWikiNote(stream.id, currentSlug);
      onClosed();
    } catch (error) {
      window.alert(`Failed to delete note: ${String(error)}`);
    }
  }, [stream.id, currentSlug, onClosed]);

  const canBack = historyIdx > 0;
  const canForward = historyIdx < history.length - 1;

  const title = summary?.title ?? currentSlug;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <header style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "6px 10px",
        borderBottom: "1px solid var(--border-subtle)",
      }}>
        <button
          type="button"
          onClick={goBack}
          disabled={!canBack}
          title={canBack ? `Back to ${history[historyIdx - 1]}` : "No previous page"}
          style={{ padding: "2px 8px" }}
        >
          ←
        </button>
        <button
          type="button"
          onClick={goForward}
          disabled={!canForward}
          title={canForward ? `Forward to ${history[historyIdx + 1]}` : "No next page"}
          style={{ padding: "2px 8px" }}
        >
          →
        </button>
        <strong style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", marginLeft: 4 }}>
          {title}
        </strong>
        {summary && <FreshnessBadge note={summary} />}
        {notFound ? (
          <button type="button" onClick={() => void handleCreate()}>Create page</button>
        ) : (
          <>
            {editing ? (
              <button type="button" onClick={enterView} title="Switch to view mode">View</button>
            ) : (
              <button type="button" onClick={enterEdit} title="Switch to edit mode">Edit</button>
            )}
            <button
              type="button"
              onClick={() => void handleSave()}
              disabled={!editing || draft === body}
              title={draft === body ? "No unsaved changes" : "Save changes"}
            >
              Save
            </button>
            <button
              type="button"
              onClick={handleRevert}
              disabled={!editing || draft === body}
              title="Discard unsaved changes"
            >
              Revert
            </button>
            <button type="button" onClick={() => void handleDelete()} title="Delete note">Delete</button>
          </>
        )}
      </header>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}>
        {notFound ? (
          <div style={{ color: "var(--text-muted)", fontSize: 13 }}>
            <div style={{ fontSize: 15, marginBottom: 8, color: "var(--text-primary)" }}>Page not found</div>
            <div>No note exists with slug <code>{currentSlug}</code>.</div>
            <div style={{ marginTop: 8 }}>
              Click <strong>Create page</strong> above to start a new note at <code>.oxplow/notes/{currentSlug}.md</code>.
            </div>
          </div>
        ) : loadError ? (
          <div style={{ color: "var(--severity-critical)" }}>Failed to load note: {loadError}</div>
        ) : editing ? (
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            style={{
              width: "100%",
              height: "100%",
              minHeight: 300,
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 13,
              background: "var(--surface-card)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-subtle)",
              padding: 8,
              resize: "none",
            }}
          />
        ) : (
          <MarkdownView
            className="wiki-note-markdown"
            body={draftInitialized ? draft : body}
            onNavigateInternal={navigate}
            onOpenInNewTab={onOpenNoteInNewTab}
            onOpenFile={(path) => onOpenFile(path)}
            renderMermaid
          />
        )}
      </div>
      {!notFound && !loadError && summary && summary.referenced_files.length > 0 && (
        <BacklinksFooter
          summary={summary}
          onOpenFile={onOpenFile}
        />
      )}
    </div>
  );
}

function BacklinksFooter({
  summary,
  onOpenFile,
}: {
  summary: WikiNoteSummary;
  onOpenFile: (path: string) => void;
}) {
  const changed = useMemo(() => new Set(summary.changed_refs), [summary.changed_refs]);
  const deleted = useMemo(() => new Set(summary.deleted_refs), [summary.deleted_refs]);
  return (
    <footer
      style={{
        borderTop: "1px solid var(--border-subtle)",
        padding: "6px 10px",
        fontSize: 12,
        color: "var(--text-muted)",
        display: "flex",
        flexWrap: "wrap",
        gap: 6,
        alignItems: "center",
      }}
    >
      <span>
        Referenced file{summary.referenced_files.length === 1 ? "" : "s"} ({summary.referenced_files.length}):
      </span>
      {summary.referenced_files.map((path) => {
        const status = deleted.has(path) ? "deleted" : changed.has(path) ? "changed" : "fresh";
        const color =
          status === "deleted"
            ? "var(--severity-critical)"
            : status === "changed"
              ? "var(--status-waiting)"
              : "var(--text-primary)";
        return (
          <button
            key={path}
            type="button"
            onClick={() => {
              if (status === "deleted") return;
              onOpenFile(path);
            }}
            disabled={status === "deleted"}
            title={
              status === "deleted"
                ? `${path} (deleted from workspace)`
                : status === "changed"
                  ? `${path} (changed since this note was written)`
                  : `Open ${path}`
            }
            style={{
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 11,
              padding: "1px 6px",
              borderRadius: 3,
              border: "1px solid var(--border-subtle)",
              background: "transparent",
              color,
              cursor: status === "deleted" ? "not-allowed" : "pointer",
              textDecoration: status === "deleted" ? "line-through" : "none",
            }}
          >
            {path}
          </button>
        );
      })}
    </footer>
  );
}

function FreshnessBadge({ note }: { note: WikiNoteSummary }) {
  const reasons = useMemo(() => {
    const r: string[] = [];
    if (note.head_advanced) r.push("HEAD advanced");
    if (note.changed_refs.length > 0) r.push(`${note.changed_refs.length} ref${note.changed_refs.length === 1 ? "" : "s"} changed`);
    if (note.deleted_refs.length > 0) r.push(`${note.deleted_refs.length} deleted`);
    return r;
  }, [note]);
  const title = reasons.length > 0 ? reasons.join("; ") : `${note.total_refs} referenced files`;
  return (
    <span
      title={title}
      style={{
        fontSize: 11,
        padding: "2px 6px",
        borderRadius: 3,
        background: FRESHNESS_COLOR[note.freshness],
        color: "#fff",
      }}
    >
      {FRESHNESS_LABEL[note.freshness]}
    </span>
  );
}

