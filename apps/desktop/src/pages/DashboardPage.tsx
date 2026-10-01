import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { PageVisitApi, Stream, TopVisitedRowApi } from "../api.js";
import { listRecentPageVisits, subscribePageVisitEvents, topVisitedPages } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import { PageKindIcon } from "../pageKinds.js";
import { useBookmarksStore } from "../tabs/useBookmarks.js";
import type { Bookmark, BookmarkScope } from "../tabs/bookmarks.js";
import { showToast } from "../components/toastStore.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "../components/RailHud/history.js";

export interface DashboardPageProps {
  stream: Stream | null;
  /** Current thread — scopes the "Go To" page's bookmark reads/writes. */
  threadId?: string | null;
  onOpenPage(ref: TabRef): void;
}

/**
 * The "Go To" page: bookmarks, recently and most visited pages. (Planning,
 * Review and Quality moved to oxplow-analytics lenses.)
 */
export function DashboardPage({ stream, threadId = null, onOpenPage }: DashboardPageProps) {
  return (
    <Page testId="page-dashboard-visits" title="Go To">
      <div style={{ padding: "16px 20px", display: "flex", flexDirection: "column", gap: 20, maxWidth: 960 }}>
        <VisitsSections stream={stream} threadId={threadId} onOpenPage={onOpenPage} />
      </div>
    </Page>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2
        style={{
          fontSize: 11,
          fontWeight: 600,
          color: "var(--text-secondary)",
          textTransform: "uppercase",
          letterSpacing: 0.4,
          margin: "0 0 8px",
        }}
      >
        {title}
      </h2>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>{children}</div>
    </section>
  );
}

/** The "Go To" page body — the universal "where do I want to go" hub:
 *  the user's bookmarks (with inline scope management + removal), the
 *  recently-visited and most-visited page lists, and a visit-volume
 *  chart. */
function VisitsSections({
  stream,
  threadId,
  onOpenPage,
}: {
  stream: Stream | null;
  threadId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [recent, setRecent] = useState<PageVisitApi[]>([]);
  const [top, setTop] = useState<TopVisitedRowApi[]>([]);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
      void listRecentPageVisits({
        threadId,
        limit: 25,
        dedupeByRef: true,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      }).then((rows) => {
        if (!cancelled) setRecent(rows);
      });
      void topVisitedPages({ limit: 25, sinceT: since }).then((rows) => {
        if (!cancelled) setTop(rows);
      });
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [threadId]);

  // Go To is purely navigational — bookmarks + plain link lists. Visit
  // analytics (counts, per-day chart) live on the Page Analytics page.
  return (
    <>
      <BookmarksManager stream={stream} threadId={threadId} onOpenPage={onOpenPage} />
      <VisitsBrowser recent={recent} top={top} onOpenPage={onOpenPage} />
    </>
  );
}

/** Single toggle-able visits list: Recently Visited vs Most Visited
 *  (last 30d), swapped via a segmented control instead of stacking. */
function VisitsBrowser({
  recent,
  top,
  onOpenPage,
}: {
  recent: PageVisitApi[];
  top: TopVisitedRowApi[];
  onOpenPage(ref: TabRef): void;
}) {
  const [mode, setMode] = useState<"recent" | "top">("recent");
  return (
    <section>
      <div style={{ display: "flex", alignItems: "center", gap: 8, margin: "0 0 8px" }}>
        <div role="tablist" aria-label="Visits view" style={{ display: "inline-flex", border: "1px solid var(--border-subtle)", borderRadius: 6, overflow: "hidden" }}>
          {([
            { key: "recent", label: "Recently Visited" },
            { key: "top", label: "Most Visited" },
          ] as const).map((opt) => {
            const active = mode === opt.key;
            return (
              <button
                key={opt.key}
                type="button"
                role="tab"
                aria-selected={active}
                data-testid={`goto-visits-mode-${opt.key}`}
                onClick={() => setMode(opt.key)}
                style={{
                  padding: "4px 12px",
                  fontSize: "var(--text-xs)",
                  background: active ? "var(--accent-soft-bg, var(--surface-app))" : "transparent",
                  color: active ? "var(--text-primary)" : "var(--text-secondary)",
                  fontWeight: active ? 600 : 400,
                  border: "none",
                  cursor: active ? "default" : "pointer",
                }}
              >
                {opt.label}
              </button>
            );
          })}
        </div>
        {mode === "top" ? (
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>Last 30 days</span>
        ) : null}
      </div>
      {mode === "recent" ? (
        recent.length === 0 ? (
          <EmptyState compact title="No visits yet" text="Pages you open show up here, most recent first." />
        ) : (
          <LinkList>
            {recent.map((r) => (
              <LinkRow
                key={r.refId}
                kind={refFromTabId(r.refId)?.kind ?? r.refKind}
                label={(r.label?.trim() ?? "") || r.refId}
                onClick={() => {
                  const ref = refFromTabId(r.refId);
                  if (ref) onOpenPage(ref);
                }}
              />
            ))}
          </LinkList>
        )
      ) : top.length === 0 ? (
        <EmptyState compact title="No visits yet" text="The pages you open most over the last 30 days show up here." />
      ) : (
        <LinkList>
          {top.map((r) => (
            <LinkRow
              key={r.refId}
              kind={r.refKind}
              label={r.label}
              onClick={() => onOpenPage({ id: r.refId, kind: r.refKind as TabRef["kind"], payload: r.payload })}
            />
          ))}
        </LinkList>
      )}
    </section>
  );
}

/** Plain unstyled list wrapper for the link rows. */
function LinkList({ children }: { children: ReactNode }) {
  return (
    <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column" }}>
      {children}
    </ul>
  );
}

const linkRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  textAlign: "left",
  padding: "7px 2px",
  background: "transparent",
  border: "none",
  borderBottom: "1px solid var(--border-subtle)",
  color: "var(--text-primary)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
};

/** A single plain text link row (no box) for the visited lists, with
 *  the page-type icon before the label (matching bookmark rows). */
function LinkRow({ kind, label, onClick }: { kind: string; label: string; onClick(): void }) {
  return (
    <li>
      <button type="button" title={label} onClick={onClick} style={linkRowStyle}>
        <PageKindIcon kind={kind} size={13} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {label}
        </span>
      </button>
    </li>
  );
}

const SCOPE_OPTIONS: { scope: BookmarkScope; letter: string; title: string }[] = [
  { scope: "thread", letter: "Thread", title: "Bookmark visible only in this thread" },
  { scope: "stream", letter: "Stream", title: "Bookmark visible across this stream" },
  { scope: "global", letter: "Global", title: "Bookmark visible everywhere" },
];

/** Bookmarks list with inline management: open, re-scope
 *  (thread / stream / global), and remove (fire-and-undo). */
function BookmarksManager({
  stream,
  threadId,
  onOpenPage,
}: {
  stream: Stream | null;
  threadId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const store = useBookmarksStore();
  const streamId = stream?.id ?? null;
  const bookmarks = store.bookmarks(threadId, streamId);

  const removeBookmark = (b: Bookmark) => {
    store.setScope(threadId, streamId, b.ref, b.label, b.scope); // collapse to a single scope first
    store.remove(b.scope, threadId, streamId, b.ref.id);
    showToast({
      message: `Removed bookmark "${b.label ?? b.ref.id}"`,
      onUndo: () => store.add(b.scope, threadId, streamId, b.ref, b.label),
    });
  };

  return (
    <Section title="Bookmarks">
      {bookmarks.length === 0 ? <EmptyState compact title="No bookmarks yet" text="Star a page to pin it here." /> : null}
      <LinkList>
      {bookmarks.map((b) => (
        <li
          key={b.ref.id}
          data-testid={`goto-bookmark-${b.ref.id}`}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "2px 2px",
            borderBottom: "1px solid var(--border-subtle)",
          }}
        >
          <button
            type="button"
            data-testid={`goto-bookmark-open-${b.ref.id}`}
            title={b.label ?? b.ref.id}
            onClick={() => onOpenPage(b.ref)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              flex: 1,
              minWidth: 0,
              background: "transparent",
              border: "none",
              color: "var(--text-primary)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
              textAlign: "left",
              padding: "7px 0",
            }}
          >
            <PageKindIcon kind={b.ref.kind} size={13} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
            <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {b.label ?? b.ref.id}
            </span>
          </button>
          <div role="group" aria-label="Bookmark scope" style={{ display: "inline-flex", border: "1px solid var(--border-subtle)", borderRadius: 6, overflow: "hidden", flexShrink: 0 }}>
            {SCOPE_OPTIONS.map((opt) => {
              const active = b.scope === opt.scope;
              return (
                <button
                  key={opt.scope}
                  type="button"
                  data-testid={`goto-bookmark-scope-${b.ref.id}-${opt.scope}`}
                  aria-pressed={active}
                  title={opt.title}
                  onClick={() => { if (!active) store.setScope(threadId, streamId, b.ref, b.label, opt.scope); }}
                  style={{
                    padding: "3px 8px",
                    fontSize: 11,
                    background: active ? "var(--accent-soft-bg, var(--surface-app))" : "transparent",
                    color: active ? "var(--text-primary)" : "var(--text-secondary)",
                    fontWeight: active ? 600 : 400,
                    border: "none",
                    cursor: active ? "default" : "pointer",
                  }}
                >
                  {opt.letter}
                </button>
              );
            })}
          </div>
          <button
            type="button"
            data-testid={`goto-bookmark-remove-${b.ref.id}`}
            title="Remove bookmark"
            aria-label="Remove bookmark"
            onClick={() => removeBookmark(b)}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-muted)",
              cursor: "pointer",
              fontSize: 14,
              lineHeight: 1,
              padding: "0 4px",
              flexShrink: 0,
            }}
          >
            ✕
          </button>
        </li>
      ))}
      </LinkList>
    </Section>
  );
}

