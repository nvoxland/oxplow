import { useEffect, useMemo, useRef } from "react";
import {
  writeWikiPageBody,
  type Stream,
  type WikiPageSummary,
} from "../../api.js";
import { MarkdownView, preprocessWikilinks, postprocessWikilinks } from "./MarkdownView.js";
import { RichTextField } from "../RichText/RichTextField.js";
import { recordOpError } from "../opErrorsStore.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";
import type { WikiPageController } from "./useWikiPageController.js";

type FreshnessStatus = WikiPageSummary["freshness"];

const FRESHNESS_LABEL: Record<NonNullable<FreshnessStatus>, string> = {
  "fresh": "fresh",
  "stale": "stale",
  "very-stale": "very stale",
};

const FRESHNESS_COLOR: Record<NonNullable<FreshnessStatus>, string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

interface Props {
  stream: Stream;
  slug: string;
  controller: WikiPageController;
  /** Published on mount so the parent can render rail content (TOC) that
   *  needs to read scroll position from the same container. */
  onScrollHostMounted?: (el: HTMLElement | null) => void;
  onNavigateInternalWikiPage: (slug: string) => void;
  onOpenWikiPageInNewTab: (slug: string) => void;
  onOpenFile: (path: string) => void;
  onOpenDirectory?: (path: string) => void;
  onOpenCommit?: (sha: string) => void;
  onOpenExternalUrl?: (url: string) => void;
}

export function WikiPageTab({
  stream,
  slug,
  controller,
  onScrollHostMounted,
  onNavigateInternalWikiPage,
  onOpenWikiPageInNewTab,
  onOpenFile,
  onOpenDirectory,
  onOpenCommit,
  onOpenExternalUrl,
}: Props) {
  const { summary, body, setDraft, notFound, loadError } = controller;
  // Pre-process `[[ ]]` wikilinks into standard markdown links before
  // handing the body to Tiptap; post-process back to `[[ ]]` form on
  // commit so the on-disk file keeps its authored shape.
  const editorValue = useMemo(() => preprocessWikilinks(body), [body]);

  // Persist scroll position across restart — see original WikiPageTab
  // for the rationale (ref-based to avoid wheel-event setState loops).
  const scrollHostRef = useRef<HTMLDivElement | null>(null);
  const scrollYRef = useRef(0);
  const pendingRestoreRef = useRef<number | null>(null);
  usePageSnapshot<{ scrollY: number }>({
    serialize: () => ({ scrollY: scrollYRef.current }),
    restore: (snap) => {
      if (typeof snap.scrollY === "number") {
        scrollYRef.current = snap.scrollY;
        pendingRestoreRef.current = snap.scrollY;
      }
    },
    deps: [body],
  });
  useEffect(() => {
    const el = scrollHostRef.current;
    if (!el) return;
    const target = pendingRestoreRef.current;
    if (target == null) return;
    if (Math.abs(el.scrollTop - target) > 1) el.scrollTop = target;
    pendingRestoreRef.current = null;
  }, [body]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div
        ref={(el) => {
          scrollHostRef.current = el;
          onScrollHostMounted?.(el);
        }}
        onScroll={(e) => { scrollYRef.current = (e.currentTarget as HTMLDivElement).scrollTop; }}
        style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}
      >
        {notFound ? (
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
            <div style={{ fontSize: "var(--text-md)", marginBottom: 8, color: "var(--text-primary)" }}>Page not found</div>
            <div>No wiki page exists with slug <code>{slug}</code>.</div>
            <div style={{ marginTop: 8 }}>
              Use <strong>Create page</strong> in the right rail to start a new wiki page at <code>.oxplow/wiki/{slug}.md</code>.
            </div>
          </div>
        ) : loadError ? (
          <div style={{ color: "var(--severity-critical)" }}>Failed to load wiki page: {loadError}</div>
        ) : (
          <RichTextField
            key={`wiki-${slug}`}
            value={editorValue}
            placeholder={`Start writing your wiki page (${slug})…`}
            comments={{
              streamId: stream.id,
              threadId: null,
              targetKind: "wiki",
              targetId: slug,
            }}
            onCommit={(markdown) => {
              const next = postprocessWikilinks(markdown);
              if (next === body) return;
              setDraft(next);
              void writeWikiPageBody(stream.id, slug, next).catch((error) => {
                recordOpError({
                  label: `Save wiki page "${slug}"`,
                  message: String(error),
                });
              });
            }}
          />
        )}
      </div>
      {!notFound && !loadError && summary && (summary.referenced_files?.length ?? 0) > 0 && (
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
  summary: WikiPageSummary;
  onOpenFile: (path: string) => void;
}) {
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else onOpenFile(path);
  };
  const changed = useMemo(() => new Set(summary.changed_refs ?? []), [summary.changed_refs]);
  const deleted = useMemo(() => new Set(summary.deleted_refs ?? []), [summary.deleted_refs]);
  const referencedFiles = summary.referenced_files ?? [];
  return (
    <footer
      style={{
        borderTop: "1px solid var(--border-subtle)",
        padding: "6px 10px",
        fontSize: "var(--text-xs)",
        color: "var(--text-muted)",
        display: "flex",
        flexWrap: "wrap",
        gap: 6,
        alignItems: "center",
      }}
    >
      <span>
        Referenced file{referencedFiles.length === 1 ? "" : "s"} ({referencedFiles.length}):
      </span>
      {referencedFiles.map((path) => {
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
              openFile(path);
            }}
            disabled={status === "deleted"}
            title={
              status === "deleted"
                ? `${path} (deleted from workspace)`
                : status === "changed"
                  ? `${path} (changed since this wiki page was written)`
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

export function FreshnessBadge({ note }: { note: WikiPageSummary }) {
  const reasons = useMemo(() => {
    const r: string[] = [];
    const changedCount = note.changed_refs?.length ?? 0;
    const deletedCount = note.deleted_refs?.length ?? 0;
    if (note.head_advanced) r.push("HEAD advanced");
    if (changedCount > 0) r.push(`${changedCount} ref${changedCount === 1 ? "" : "s"} changed`);
    if (deletedCount > 0) r.push(`${deletedCount} deleted`);
    return r;
  }, [note]);
  const totalRefs = note.total_refs ?? note.referenced_files?.length ?? 0;
  const title = reasons.length > 0 ? reasons.join("; ") : `${totalRefs} referenced files`;
  const freshness = note.freshness ?? "fresh";
  return (
    <span
      title={title}
      style={{
        fontSize: 11,
        padding: "2px 6px",
        borderRadius: 3,
        background: FRESHNESS_COLOR[freshness],
        color: "#fff",
      }}
    >
      {FRESHNESS_LABEL[freshness]}
    </span>
  );
}
