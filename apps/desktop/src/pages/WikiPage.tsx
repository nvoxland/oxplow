import { useEffect, useState } from "react";
import type { Stream, ThreadWorkState } from "../tauri-bridge/index.js";
import { commands } from "../tauri-bridge/index.js";
import { Page } from "../tabs/Page.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { WikiPageTab, FreshnessBadge } from "../components/Wiki/WikiPageTab.js";
import { useWikiPageController } from "../components/Wiki/useWikiPageController.js";
import { WikiTableOfContents } from "../components/Wiki/WikiTableOfContents.js";
import type { TabRef } from "../tabs/tabState.js";
import { wikiFreshnessRef, wikiPageRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { usePageTitle, useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";

export interface WikiPageProps {
  stream: Stream | null;
  slug: string;
  threadWork: ThreadWorkState | null;
  onClosed(): void;
  onOpenWikiPage(slug: string): void;
  onOpenFile(path: string): void;
  onOpenDirectory?(path: string): void;
  onOpenPage(ref: TabRef): void;
  onOpenCommit?(sha: string): void;
  onOpenExternalUrl?(url: string): void;
}

/**
 * Thin Page wrapper around `WikiPageTab`. Owns the wiki-page controller
 * so the body and the right rail can share state. The shared chrome owns
 * the title (via `usePageTitle`), the back/forward nav bar, and the
 * bookmark toggle. In-tab wikilink clicks route through the navigation
 * context so they participate in tab-level history.
 */
export function WikiPage({ stream, slug, onClosed, onOpenWikiPage, onOpenFile, onOpenDirectory, onOpenPage, onOpenCommit, onOpenExternalUrl }: WikiPageProps) {
  const nav = useOptionalPageNavigation();
  const ref = wikiPageRef(slug);
  const backlinkEntries = useBacklinks(ref);
  const outboundEntries = usePageOutbound(ref);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} />,
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
      onOpenWikiPage={onOpenWikiPage}
      onOpenFile={onOpenFile}
      onOpenDirectory={onOpenDirectory}
      onOpenPage={onOpenPage}
      onOpenCommit={onOpenCommit}
      onOpenExternalUrl={onOpenExternalUrl}
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
  onOpenWikiPage,
  onOpenFile,
  onOpenDirectory,
  onOpenCommit,
  onOpenExternalUrl,
  backlinks,
  outbound,
  nav,
}: {
  stream: Stream;
  slug: string;
  onClosed: () => void;
  onOpenWikiPage: (slug: string) => void;
  onOpenFile: (path: string) => void;
  onOpenDirectory?: (path: string) => void;
  onOpenPage: (ref: TabRef) => void;
  onOpenCommit?: (sha: string) => void;
  onOpenExternalUrl?: (url: string) => void;
  backlinks: { count: number; body: React.ReactNode };
  outbound: { count: number; body: React.ReactNode } | undefined;
  nav: ReturnType<typeof useOptionalPageNavigation>;
}) {
  const controller = useWikiPageController(stream, slug, onClosed);
  usePageTitle(controller.summary?.title ?? slug);
  const [scrollHost, setScrollHost] = useState<HTMLElement | null>(null);
  const [staleCount, setStaleCount] = useState<number | null>(null);
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await commands.listWikiFreshness(slug);
      if (cancelled) return;
      if (r.status === "ok") {
        setStaleCount(r.data.filter((row) => row.stale).length);
      } else {
        setStaleCount(null);
      }
    })();
    return () => { cancelled = true; };
  }, [slug, controller.summary?.updated_at]);
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
      chips={staleCount != null && staleCount > 0 ? [{
        label: `${staleCount} stale ref${staleCount === 1 ? "" : "s"}`,
        color: "var(--priority-high)",
        title: "Click to open Freshness view",
      }] : undefined}
      actions={
        <button
          type="button"
          onClick={openFreshness}
          style={{
            background: "transparent",
            border: "1px solid var(--border-subtle)",
            borderRadius: 4,
            color: staleCount && staleCount > 0 ? "var(--priority-high)" : "var(--text-secondary)",
            padding: "2px 8px",
            fontSize: "var(--text-xs)",
            cursor: "pointer",
          }}
          title="Open the freshness view for this wiki page"
        >
          Freshness{staleCount != null ? ` (${staleCount} stale)` : ""}
        </button>
      }
      backlinks={backlinks}
      outbound={outbound}
      commentsNav={<CommentNavigator targetKind="wiki" targetId={slug} />}
      layout="details"
      rightRail={rail}
    >
      <WikiPageTab
        stream={stream}
        slug={slug}
        controller={controller}
        onScrollHostMounted={setScrollHost}
        onNavigateInternalWikiPage={(nextSlug) => nav ? nav.navigate(wikiPageRef(nextSlug)) : onOpenWikiPage(nextSlug)}
        onOpenWikiPageInNewTab={onOpenWikiPage}
        onOpenFile={onOpenFile}
        onOpenDirectory={onOpenDirectory}
        onOpenCommit={onOpenCommit}
        onOpenExternalUrl={onOpenExternalUrl}
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
  const { summary, body, notFound, loadError, create, remove } = controller;

  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      minHeight: "100%",
      gap: 16,
      padding: "4px 0",
    }}>
      {summary && (
        <div style={{ display: "flex" }}>
          <FreshnessBadge note={summary} />
        </div>
      )}

      {!notFound && !loadError && (
        <WikiTableOfContents bodyText={body} scrollHost={scrollHost} />
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
          <RailButton onClick={() => void remove()} variant="danger">Delete</RailButton>
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
