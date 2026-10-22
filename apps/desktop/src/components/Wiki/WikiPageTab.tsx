import type { PageChip } from "../../tabs/Page.js";
import { useEffect, useRef } from "react";
import {
  writeWikiPage,
  type Stream,
} from "../../api.js";
import type { WikiRefFreshness } from "../../knowledge.js";
import {
  type WikiFreshnessLevel,
  type WikiFreshnessSummary,
} from "./wikiFreshness.js";
import { contextNodeProps } from "../Comments/contextNodes.js";
import { Markdown } from "../Markdown/Markdown.js";
import { recordOpError } from "../../opErrors.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";
import { useHistoryState } from "../../tabs/useHistoryState.js";
import type { WikiPageController } from "./useWikiPageController.js";
import { wikiUri } from "../../uri/uri.js";

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
  /** Per-ref freshness rows from `v_knowledge_ref` (`readWikiFreshness`) — the page's one
   *  freshness source (fetched by WikiPage, shared with the header chip
   *  and rail badge). Drives the referenced-files footer. */
  freshnessRows?: WikiRefFreshness[];
  /** Published on mount so the parent can render rail content (TOC) that
   *  needs to read scroll position from the same container. */
  onScrollHostMounted?: (el: HTMLElement | null) => void;
  /** The referenced-files footer's file links. */
  onOpenFile: (path: string) => void;
}

export function WikiPageTab({
  stream,
  slug,
  controller,
  freshnessRows,
  onScrollHostMounted,
  onOpenFile,
}: Props) {
  const { body, setDraft, notFound, loadError } = controller;

  // Keep the scroll position with the tab's entry (Back, a restart). The
  // position lives in a ref while scrolling, so a wheel doesn't set state;
  // it is written once the scroll settles and when the page goes away.
  const [savedY, setSavedY] = useHistoryState<number>("scrollY", 0);
  const scrollHostRef = useRef<HTMLDivElement | null>(null);
  const scrollYRef = useRef(savedY);
  const pendingRestoreRef = useRef<number | null>(savedY > 0 ? savedY : null);
  const saveScrollRef = useRef(setSavedY);
  saveScrollRef.current = setSavedY;
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (scrollTimerRef.current) {
        clearTimeout(scrollTimerRef.current);
        saveScrollRef.current(scrollYRef.current);
      }
    },
    [],
  );
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
        onScroll={(e) => {
          scrollYRef.current = (e.currentTarget as HTMLDivElement).scrollTop;
          if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current);
          scrollTimerRef.current = setTimeout(() => {
            scrollTimerRef.current = null;
            saveScrollRef.current(scrollYRef.current);
          }, 200);
        }}
        style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}
        // The body is a wiki context node so any plain-DOM selection in
        // it (and the page_ref graph) resolves to its wiki URI; the
        // Tiptap surface inside carries its own comment affordance.
        {...contextNodeProps(wikiUri(slug))}
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
          <Markdown
            key={`wiki-${slug}`}
            value={body}
            placeholder={`Start writing your wiki page (${slug})…`}
            comments={{
              streamId: stream.id,
              threadId: null,
              target: wikiUri(slug),
            }}
            onCommit={(next) => {
              if (next === body) return;
              setDraft(next);
              void writeWikiPage(slug, next).catch((error) => {
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
/// `v_knowledge_ref` rows (`summarizeWikiFreshness`) — the same
/// source the header chip counts, so the two can't disagree.
/** A wiki page's freshness as its page's chip: stale refs say so on hover,
 *  and pressing it opens the freshness report. */
export function freshnessChip(summary: WikiFreshnessSummary, open: () => void): PageChip {
  const refs =
    summary.staleRefs.length > 0
      ? `${summary.staleRefs.length} of ${summary.totalRefs} ref${summary.totalRefs === 1 ? "" : "s"} stale`
      : `${summary.totalRefs} referenced file${summary.totalRefs === 1 ? "" : "s"}`;
  return {
    label: FRESHNESS_LABEL[summary.freshness],
    background: FRESHNESS_COLOR[summary.freshness],
    color: "#fff",
    title: `${refs}. Open the freshness view for this wiki page.`,
    onClick: open,
  };
}
