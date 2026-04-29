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
import { recordOpError } from "../opErrorsStore.js";
import { usePageTitle } from "../../tabs/PageNavigationContext.js";

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
  /** Called for plain in-tab wikilink-to-note navigation. Routes through
   *  the host's PageNavigationContext so back/forward live in the shared
   *  chrome rather than a per-NoteTab history. */
  onNavigateInternalNote: (slug: string) => void;
  onOpenNoteInNewTab: (slug: string) => void;
  onOpenFile: (path: string) => void;
  /** Optional handler for git-commit wikilink clicks — opens the
   *  GitCommitPage for the SHA. */
  onOpenCommit?: (sha: string) => void;
  /** Optional handler for external (http/https) link clicks — host opens
   *  it as an in-app external-url tab. Falls back to OS browser when
   *  unset. */
  onOpenExternalUrl?: (url: string) => void;
}

export function NoteTab({ stream, slug, onClosed, onNavigateInternalNote, onOpenNoteInNewTab, onOpenFile, onOpenCommit, onOpenExternalUrl }: Props) {
  const [summary, setSummary] = useState<WikiNoteSummary | null>(null);
  const [body, setBody] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);

  // Title flows through the shared PageNavigationContext so it surfaces in
  // the chrome header and the tab strip without a duplicate row inside the
  // note body. Falls back to the slug until the summary loads.
  usePageTitle(summary?.title ?? slug);

  const refresh = useCallback(async () => {
    try {
      const all = await listWikiNotes(stream.id);
      setSummary(all.find((n) => n.slug === slug) ?? null);
    } catch {}
    try {
      const text = await readWikiNoteBody(stream.id, slug);
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
  }, [stream.id, slug]);

  useEffect(() => {
    void refresh();
    setEditing(false);
  }, [refresh]);

  useEffect(() => subscribeWikiNoteEvents(() => { void refresh(); }), [refresh]);

  const [draftInitialized, setDraftInitialized] = useState(false);

  useEffect(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
  }, [body, draftInitialized]);

  useEffect(() => {
    setDraftInitialized(false);
  }, [slug]);

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
      await writeWikiNoteBody(stream.id, slug, draft);
      setBody(draft);
    } catch (error) {
      recordOpError({
        label: `Save note "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug, draft]);

  const handleCreate = useCallback(async () => {
    const seed = `# ${slug}\n\n`;
    try {
      await writeWikiNoteBody(stream.id, slug, seed);
      setNotFound(false);
      setBody(seed);
      setDraft(seed);
      setDraftInitialized(true);
      setEditing(true);
    } catch (error) {
      recordOpError({
        label: `Create note "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug]);

  const handleDelete = useCallback(async () => {
    if (!window.confirm(`Delete note "${slug}"? The file will be removed.`)) return;
    try {
      await deleteWikiNote(stream.id, slug);
      onClosed();
    } catch (error) {
      recordOpError({
        label: `Delete note "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug, onClosed]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          padding: "4px 12px",
          borderBottom: "1px solid var(--border-subtle)",
          background: "var(--surface-app)",
          fontSize: 12,
          flexShrink: 0,
        }}
      >
        {summary && <FreshnessBadge note={summary} />}
        <div style={{ flex: 1 }} />
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
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}>
        {notFound ? (
          <div style={{ color: "var(--text-muted)", fontSize: 13 }}>
            <div style={{ fontSize: 15, marginBottom: 8, color: "var(--text-primary)" }}>Page not found</div>
            <div>No note exists with slug <code>{slug}</code>.</div>
            <div style={{ marginTop: 8 }}>
              Click <strong>Create page</strong> above to start a new note at <code>.oxplow/notes/{slug}.md</code>.
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
            body={stripLeadingH1(draftInitialized ? draft : body)}
            onNavigateInternal={onNavigateInternalNote}
            onOpenInNewTab={onOpenNoteInNewTab}
            onOpenFile={(path) => onOpenFile(path)}
            onOpenCommit={onOpenCommit}
            onOpenExternalUrl={onOpenExternalUrl}
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

/**
 * Drop a leading `# heading` line from the rendered note body. The
 * page chrome already shows the note title in the nav bar, so showing
 * it again as the first markdown row would duplicate the line the
 * user just read above.
 */
function stripLeadingH1(body: string): string {
  const match = body.match(/^\s*#\s+[^\n]*\n+/);
  return match ? body.slice(match[0].length) : body;
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
