import { useCallback, useEffect, useState } from "react";
import { InlineConfirm } from "../components/InlineConfirm.js";
import type { Stream } from "../tauri-bridge/index.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import type { WikiRefFreshness } from "../knowledge.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import type { WorkList } from "../workItems.js";
import { readWikiFreshness } from "../knowledge.js";
import { summarizeWikiFreshness } from "../components/Wiki/wikiFreshness.js";
import { Page } from "../tabs/Page.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { WikiPageTab, freshnessChip } from "../components/Wiki/WikiPageTab.js";
import { useWikiPageController } from "../components/Wiki/useWikiPageController.js";
import { Contents } from "../lens/Contents.js";
import type { TabRef } from "../tabs/tabState.js";
import { wikiFreshnessRef, wikiPageRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { usePageTitle, useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { wikiUri } from "../uri/uri.js";

export interface WikiPageProps {
  stream: Stream | null;
  slug: string;
  threadWork: WorkList | null;
  onClosed(): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
}

/**
 * Thin Page wrapper around `WikiPageTab`. Owns the wiki-page controller
 * so the body and the right rail can share state. The shared chrome owns
 * the title (via `usePageTitle`), the back/forward nav bar, and the
 * bookmark toggle. In-tab wikilink clicks route through the navigation
 * context so they participate in tab-level history.
 */
export function WikiPage({ stream, slug, onClosed, onOpenFile, onOpenPage }: WikiPageProps) {
  const nav = useOptionalPageNavigation();
  const ref = wikiPageRef(slug);
  const backlinkEntries = useBacklinks(ref);
  const outboundEntries = usePageOutbound(ref);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} />,
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} />,
        }
      : undefined;

  if (!stream) {
    return (
      <Page testId="page-wiki" kind="wiki" backlinks={backlinks} outbound={outbound}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
          No stream selected.
        </div>
      </Page>
    );
  }
  return (
    <WikiPageBody
      stream={stream}
      slug={slug}
      onClosed={onClosed}
      onOpenFile={onOpenFile}
      onOpenPage={onOpenPage}
      backlinks={backlinks}
      outbound={outbound}
      nav={nav}
    />
  );
}

function WikiPageBody({
  stream,
  slug,
  onClosed,
  onOpenFile,
  backlinks,
  outbound,
  nav,
}: {
  stream: Stream;
  slug: string;
  onClosed: () => void;
  onOpenFile: (path: string) => void;
  onOpenPage: (ref: TabRef) => void;
  backlinks: { count: number; body: React.ReactNode };
  outbound: { count: number; body: React.ReactNode } | undefined;
  nav: ReturnType<typeof useOptionalPageNavigation>;
}) {
  const controller = useWikiPageController(slug, onClosed);
  usePageTitle(controller.summary?.title ?? slug);
  const [scrollHost, setScrollHost] = useState<HTMLElement | null>(null);
  // The ONE freshness fetch for the page — header chip, rail badge, and
  // referenced-files footer all derive from these rows (see
  // wikiFreshness.ts for why splitting sources is forbidden).
  const [freshnessRows, setFreshnessRows] = useState<WikiRefFreshness[] | null>(null);
  const [freshnessReads, setFreshnessReads] = useState<Reads>(NO_READS);
  const loadFreshness = useCallback(() => {
    let cancelled = false;
    void readWikiFreshness(slug)
      .then(({ rows, reads }) => {
        if (cancelled) return;
        setFreshnessRows(rows);
        setFreshnessReads(reads);
      })
      .catch(() => {
        if (!cancelled) setFreshnessRows(null);
      });
    return () => {
      cancelled = true;
    };
  }, [slug]);
  useEffect(() => loadFreshness(), [loadFreshness]);
  // A drifting ref (a new snapshot) changes `v_knowledge_ref.stale`
  // without touching the page row: re-read on what the read read.
  useRerunOnChange(freshnessReads, () => void loadFreshness());
  const freshness = freshnessRows != null ? summarizeWikiFreshness(freshnessRows) : null;
  const openFreshness = () => {
    const ref = wikiFreshnessRef(slug);
    if (nav) nav.navigate(ref);
  };

  const rail = (
    <WikiPageRail
      controller={controller}
      scrollHost={scrollHost}
    />
  );

  return (
    <Page
      testId="page-wiki"
      kind="wiki"
      chips={freshness ? [freshnessChip(freshness, openFreshness)] : undefined}
      backlinks={backlinks}
      outbound={outbound}
      commentsNav={<CommentNavigator target={wikiUri(slug)} />}
      width="reading"
      rightRail={rail}
    >
      <WikiPageTab
        stream={stream}
        slug={slug}
        controller={controller}
        freshnessRows={freshnessRows ?? undefined}
        onScrollHostMounted={setScrollHost}
        onOpenFile={onOpenFile}
      />
    </Page>
  );
}

function WikiPageRail({
  controller,
  scrollHost,
}: {
  controller: ReturnType<typeof useWikiPageController>;
  scrollHost: HTMLElement | null;
}) {
  const { summary, notFound, loadError, create, remove } = controller;

  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      minHeight: "100%",
      gap: 16,
      padding: "4px 0",
    }}>
      {!notFound && !loadError && (
        <Contents root={scrollHost} />
      )}

      {summary && (
        <div style={{
          display: "flex",
          flexDirection: "column",
          gap: 4,
          fontSize: "var(--text-xs)",
          color: "var(--text-muted)",
        }}>
          <div style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: "0.06em",
            color: "var(--text-muted)",
          }}>Last edited</div>
          <div style={{ color: "var(--text-secondary)" }}>{formatTimestamp(summary.updated_at)}</div>
        </div>
      )}

      <div style={{ flex: 1 }} />

      <div style={{
        display: "flex",
        flexDirection: "column",
        gap: 6,
        borderTop: "1px solid var(--border-subtle)",
        paddingTop: 12,
      }}>
        {notFound ? (
          <RailButton onClick={() => void create()} variant="primary">Create page</RailButton>
        ) : loadError ? null : (
          // Asks inline: the first press arms it, Confirm deletes.
          <InlineConfirm onConfirm={() => void remove()} confirmLabel="Delete Page" testIdPrefix="wiki-rail-delete">
            {(arm) => <RailButton onClick={arm} variant="danger">Delete</RailButton>}
          </InlineConfirm>
        )}
      </div>
    </div>
  );
}

function RailButton({
  onClick,
  disabled,
  variant,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  variant?: "primary" | "danger";
  children: React.ReactNode;
}) {
  const isPrimary = variant === "primary";
  const isDanger = variant === "danger";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={{
        textAlign: "left",
        padding: "6px 10px",
        borderRadius: 4,
        border: "1px solid var(--border-subtle)",
        background: isPrimary
          ? "var(--accent-soft-bg, var(--surface-elevated))"
          : "var(--surface-card)",
        color: disabled
          ? "var(--text-muted)"
          : isDanger
            ? "var(--severity-critical)"
            : isPrimary
              ? "var(--text-primary)"
              : "var(--text-secondary)",
        cursor: disabled ? "not-allowed" : "pointer",
        fontSize: "var(--text-xs)",
        fontWeight: isPrimary ? 600 : 400,
        opacity: disabled ? 0.6 : 1,
      }}
    >
      {children}
    </button>
  );
}

function formatTimestamp(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleString();
  } catch {
    return iso;
  }
}
