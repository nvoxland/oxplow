import { useEffect, useMemo, useRef } from "react";
import {
  writeWikiPageBody,
  type Stream,
} from "../../api.js";
import type { WikiRefFreshness } from "../../tauri-bridge/generated/bindings.js";
import {
  type WikiFreshnessLevel,
  type WikiFreshnessSummary,
} from "./wikiFreshness.js";
import { MarkdownView, preprocessWikilinks, postprocessWikilinks } from "./MarkdownView.js";
import { contextNodeProps } from "../Comments/contextNodes.js";
import { RichTextField } from "../RichText/RichTextField.js";
import { recordOpError } from "../opErrorsStore.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";
import type { WikiPageController } from "./useWikiPageController.js";

const FRESHNESS_LABEL: Record<WikiFreshnessLevel, string> = {
  "fresh": "fresh",
  "stale": "stale",
  "very-stale": "very stale",
};

const FRESHNESS_COLOR: Record<WikiFreshnessLevel, string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

interface Props {
  stream: Stream;
  slug: string;
  controller: WikiPageController;
  /** Per-ref freshness rows from `list_wiki_freshness` — the page's one
   *  freshness source (fetched by WikiPage, shared with the header chip
   *  and rail badge). Drives the referenced-files footer. */
  freshnessRows?: WikiRefFreshness[];
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
  freshnessRows,
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
        // The body is a wiki context node so any plain-DOM selection in
        // it (and the page_ref graph) resolves to `wiki:<slug>`; the
        // Tiptap surface inside carries its own comment affordance.
        {...contextNodeProps("wiki", slug)}
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
      {!notFound && !loadError && (freshnessRows?.length ?? 0) > 0 && (
        <ReferencedFilesFooter
          rows={freshnessRows ?? []}
          onOpenFile={onOpenFile}
        />
      )}
    </div>
  );
}

function ReferencedFilesFooter({
  rows,
  onOpenFile,
}: {
  rows: WikiRefFreshness[];
  onOpenFile: (path: string) => void;
}) {
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else onOpenFile(path);
  };
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
        Referenced file{rows.length === 1 ? "" : "s"} ({rows.length}):
      </span>
      {rows.map((row) => (
        <button
          key={row.path}
          type="button"
          onClick={() => openFile(row.path)}
          title={
            row.stale
              ? `${row.path} (changed since this wiki page was written)`
              : `Open ${row.path}`
          }
          style={{
            fontFamily: "var(--font-mono, monospace)",
            fontSize: 11,
            padding: "1px 6px",
            borderRadius: 3,
            border: "1px solid var(--border-subtle)",
            background: "transparent",
            color: row.stale ? "var(--status-waiting)" : "var(--text-primary)",
            cursor: "pointer",
          }}
        >
          {row.path}
        </button>
      ))}
    </footer>
  );
}

/// Renders the freshness level derived from the page's
/// `list_wiki_freshness` rows (`summarizeWikiFreshness`) — the same
/// source the header chip counts, so the two can't disagree.
export function FreshnessBadge({ summary }: { summary: WikiFreshnessSummary }) {
  const title =
    summary.staleRefs.length > 0
      ? `${summary.staleRefs.length} of ${summary.totalRefs} ref${summary.totalRefs === 1 ? "" : "s"} stale`
      : `${summary.totalRefs} referenced file${summary.totalRefs === 1 ? "" : "s"}`;
  return (
    <span
      title={title}
      style={{
        fontSize: 11,
        padding: "2px 6px",
        borderRadius: 3,
        background: FRESHNESS_COLOR[summary.freshness],
        color: "#fff",
      }}
    >
      {FRESHNESS_LABEL[summary.freshness]}
    </span>
  );
}
