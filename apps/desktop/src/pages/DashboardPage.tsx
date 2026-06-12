import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { BacklogState, CodeQualityFindingRow, CountByDayRowApi, FileSnapshot, Stream, ThreadWorkState, TopVisitedRowApi, WikiPageSummary, Task } from "../api.js";
import {
  countPageVisitsByDay,
  listCodeQualityFindings,
  listFileSnapshots,
  listWikiPages,
  subscribePageVisitEvents,
  topVisitedPages,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { findingRef, indexRef, wikiPageRef, taskRef } from "../tabs/pageRefs.js";
import { useRouteDispatch } from "../tabs/RouteLink.js";

export type DashboardVariant = "planning" | "review" | "quality" | "visits";

export interface DashboardPageProps {
  variant: DashboardVariant;
  stream: Stream | null;
  threadWork: ThreadWorkState | null;
  backlog: BacklogState | null;
  onOpenPage(ref: TabRef): void;
}

const VARIANT_TITLE: Record<DashboardVariant, string> = {
  planning: "Planning",
  review: "Review",
  quality: "Quality",
  visits: "Visits",
};

/**
 * Composite dashboard pages — Planning, Review, Quality. Each is a
 * read-only summary stitched together from existing data slices: no new
 * IPC, no new mutations, just buttons that route through `onOpenPage`.
 */
export function DashboardPage({ variant, stream, threadWork, backlog, onOpenPage }: DashboardPageProps) {
  return (
    <Page testId={`page-dashboard-${variant}`} title={VARIANT_TITLE[variant]}>
      <div style={{ padding: "16px 20px", display: "flex", flexDirection: "column", gap: 20, maxWidth: 960 }}>
        {variant === "planning" ? (
          <PlanningSections threadWork={threadWork} backlog={backlog} stream={stream} onOpenPage={onOpenPage} />
        ) : null}
        {variant === "review" ? (
          <ReviewSections threadWork={threadWork} stream={stream} onOpenPage={onOpenPage} />
        ) : null}
        {variant === "quality" ? (
          <QualitySections stream={stream} onOpenPage={onOpenPage} />
        ) : null}
        {variant === "visits" ? (
          <VisitsSections onOpenPage={onOpenPage} />
        ) : null}
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

function RowButton({
  label,
  subtitle,
  onClick,
  testId,
  navRef,
  siblings,
  onNavigate,
}: {
  label: string;
  subtitle?: string;
  onClick?(): void;
  testId?: string;
  /** When supplied, the row dispatches via `useRouteDispatch` so it
   *  participates in in-tab navigation + sibling navigation. The
   *  legacy `onClick` is used as the rail-side fallback when no page
   *  context exists. */
  navRef?: TabRef;
  siblings?: import("../tabs/PageNavigationContext.js").NavSiblings;
  onNavigate?(ref: TabRef, opts?: { newTab?: boolean }): void;
}) {
  // Hooks must run unconditionally; pass a placeholder ref when
  // navRef is omitted and rely on the caller's onClick.
  const dispatchHook = useRouteDispatch(navRef ?? indexRef("settings"), {
    siblings,
    onNavigate: onNavigate ?? (() => onClick?.()),
  });
  const handleClick = () => {
    if (navRef) dispatchHook.dispatch(false);
    else onClick?.();
  };
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={handleClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "8px 10px",
        background: "var(--surface-tab-inactive)",
        color: "var(--text-primary)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        cursor: "pointer",
        fontSize: "var(--text-sm)",
        textAlign: "left",
      }}
    >
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {label}
      </span>
      {subtitle ? (
        <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{subtitle}</span>
      ) : null}
    </button>
  );
}

function EmptyHint({ children }: { children: ReactNode }) {
  return (
    <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)", fontStyle: "italic" }}>{children}</div>
  );
}

function useRecentNotes(stream: Stream | null) {
  const [notes, setNotes] = useState<WikiPageSummary[]>([]);
  useEffect(() => {
    if (!stream) {
      setNotes([]);
      return;
    }
    let cancelled = false;
    void listWikiPages(stream.id).then((rows) => {
      if (!cancelled) {
        const sorted = [...rows].sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1));
        setNotes(sorted);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [stream?.id]);
  return notes;
}

function useFindings(stream: Stream | null) {
  const [rows, setRows] = useState<CodeQualityFindingRow[]>([]);
  useEffect(() => {
    if (!stream) {
      setRows([]);
      return;
    }
    let cancelled = false;
    void listCodeQualityFindings({ streamId: stream.id }).then((r) => {
      if (!cancelled) setRows(r);
    });
    return () => {
      cancelled = true;
    };
  }, [stream?.id]);
  return rows;
}

function useRecentSnapshots(stream: Stream | null) {
  const [snaps, setSnaps] = useState<FileSnapshot[]>([]);
  useEffect(() => {
    if (!stream) {
      setSnaps([]);
      return;
    }
    let cancelled = false;
    void listFileSnapshots(stream.id).then((rows) => {
      if (!cancelled) {
        const sorted = [...rows].sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
        setSnaps(sorted.slice(0, 10));
      }
    });
    return () => {
      cancelled = true;
    };
  }, [stream?.id]);
  return snaps;
}

function PlanningSections({
  threadWork,
  backlog,
  stream,
  onOpenPage,
}: {
  threadWork: ThreadWorkState | null;
  backlog: BacklogState | null;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const ready: Task[] = threadWork?.waiting ?? [];
  const backlogItems = backlog?.items ?? [];
  const notes = useRecentNotes(stream);

  return (
    <>
      <Section title="Ready in This Thread">
        {ready.length === 0 ? <EmptyHint>Nothing ready.</EmptyHint> : null}
        {(() => {
          const list = ready.slice(0, 10);
          const siblingEntries = list.map((it) => ({ ref: taskRef(it.id), label: it.title }));
          return list.map((item, i) => (
            <RowButton
              key={item.id}
              testId={`dashboard-planning-ready-${item.id}`}
              label={item.title}
              subtitle={item.priority}
              navRef={taskRef(item.id)}
              siblings={{ entries: siblingEntries, index: i, title: "Ready in This Thread" }}
              onNavigate={(ref) => onOpenPage(ref)}
              onClick={() => onOpenPage(taskRef(item.id))}
            />
          ));
        })()}
      </Section>
      <Section title="Backlog">
        {backlogItems.length === 0 ? <EmptyHint>Backlog is empty.</EmptyHint> : null}
        {(() => {
          const list = backlogItems.slice(0, 10);
          const siblingEntries = list.map((it) => ({ ref: taskRef(it.id), label: it.title }));
          return list.map((item, i) => (
            <RowButton
              key={item.id}
              label={item.title}
              subtitle={item.priority}
              navRef={taskRef(item.id)}
              siblings={{ entries: siblingEntries, index: i, title: "Backlog" }}
              onNavigate={(ref) => onOpenPage(ref)}
              onClick={() => onOpenPage(taskRef(item.id))}
            />
          ));
        })()}
      </Section>
      <Section title="Recent Notes">
        {notes.length === 0 ? <EmptyHint>No notes yet.</EmptyHint> : null}
        {(() => {
          const list = notes.slice(0, 8);
          const siblingEntries = list.map((n) => ({ ref: wikiPageRef(n.slug), label: n.title || n.slug }));
          return list.map((note, i) => (
            <RowButton
              key={note.slug}
              label={note.title || note.slug}
              navRef={wikiPageRef(note.slug)}
              siblings={{ entries: siblingEntries, index: i, title: "Recent Notes" }}
              onNavigate={(ref) => onOpenPage(ref)}
              onClick={() => onOpenPage(wikiPageRef(note.slug))}
            />
          ));
        })()}
      </Section>
      <Section title="Subsystem Docs">
        <RowButton
          label="Open subsystem docs index"
          onClick={() => onOpenPage(indexRef("subsystem-docs"))}
        />
      </Section>
    </>
  );
}

function ReviewSections({
  stream,
  onOpenPage,
}: {
  threadWork: ThreadWorkState | null;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const snaps = useRecentSnapshots(stream);
  const findings = useFindings(stream);

  return (
    <>
      <Section title="Recent Snapshots">
        {snaps.length === 0 ? <EmptyHint>No snapshots yet.</EmptyHint> : null}
        {snaps.map((snap) => (
          <RowButton
            key={snap.id}
            label={snap.label ?? snap.source}
            subtitle={new Date(snap.created_at).toLocaleString()}
            onClick={() => onOpenPage(indexRef("local-history"))}
          />
        ))}
      </Section>
      <Section title="New Findings">
        {findings.length === 0 ? <EmptyHint>No findings recorded.</EmptyHint> : null}
        {findings.slice(0, 10).map((f) => (
          <RowButton
            key={f.id}
            label={`${f.kind} in ${f.path}`}
            subtitle={`metric ${f.metricValue}`}
            onClick={() => onOpenPage(findingRef(String(f.id)))}
          />
        ))}
      </Section>
    </>
  );
}

function QualitySections({ stream, onOpenPage }: { stream: Stream | null; onOpenPage(ref: TabRef): void }) {
  const findings = useFindings(stream);
  const complexity = useMemo(
    () => findings.filter((f) => f.kind === "complexity").sort((a, b) => b.metricValue - a.metricValue).slice(0, 10),
    [findings],
  );
  const dupes = useMemo(() => findings.filter((f) => f.kind === "duplicate-block"), [findings]);

  return (
    <>
      <Section title="All Findings">
        {findings.length === 0 ? <EmptyHint>No findings recorded yet — run a scan from the Code quality page.</EmptyHint> : null}
        {findings.slice(0, 20).map((f) => (
          <RowButton
            key={f.id}
            label={`${f.kind} in ${f.path}`}
            subtitle={`metric ${f.metricValue}`}
            onClick={() => onOpenPage(findingRef(String(f.id)))}
          />
        ))}
      </Section>
      <Section title="Complexity Outliers">
        {complexity.length === 0 ? <EmptyHint>No complexity findings.</EmptyHint> : null}
        {complexity.map((f) => (
          <RowButton
            key={f.id}
            label={`${f.path} (lines ${f.startLine}–${f.endLine})`}
            subtitle={`CCN ${f.metricValue}`}
            onClick={() => onOpenPage(findingRef(String(f.id)))}
          />
        ))}
      </Section>
      <Section title="Duplicate Blocks">
        {dupes.length === 0 ? <EmptyHint>No duplicate blocks reported.</EmptyHint> : null}
        {dupes.slice(0, 10).map((f) => (
          <RowButton
            key={f.id}
            label={f.path}
            subtitle={`${f.endLine - f.startLine + 1} lines`}
            onClick={() => onOpenPage(findingRef(String(f.id)))}
          />
        ))}
      </Section>
    </>
  );
}

function VisitsSections({ onOpenPage }: { onOpenPage(ref: TabRef): void }) {
  const [top, setTop] = useState<TopVisitedRowApi[]>([]);
  const [byDay, setByDay] = useState<CountByDayRowApi[]>([]);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
      void topVisitedPages({ limit: 25, sinceT: since }).then((rows) => {
        if (!cancelled) setTop(rows);
      });
      void countPageVisitsByDay({ sinceT: since }).then((rows) => {
        if (!cancelled) setByDay(rows);
      });
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  return (
    <>
      <Section title="Visits per Day (Last 30d)">
        <DailyChart rows={byDay} />
      </Section>
      <Section title="Top 25 Most Visited (Last 30d)">
        {top.length === 0 ? <EmptyHint>No visits recorded yet.</EmptyHint> : null}
        {top.map((r) => (
          <RowButton
            key={r.refId}
            label={r.label}
            subtitle={`${r.count} visit${r.count === 1 ? "" : "s"} · ${r.refKind} · last ${formatRelative(r.lastT)}`}
            onClick={() => onOpenPage({ id: r.refId, kind: r.refKind as TabRef["kind"], payload: r.payload })}
          />
        ))}
      </Section>
    </>
  );
}

function DailyChart({ rows }: { rows: CountByDayRowApi[] }) {
  if (rows.length === 0) {
    return <EmptyHint>No visits in the last 30 days.</EmptyHint>;
  }
  const max = Math.max(1, ...rows.map((r) => r.count));
  const total = rows.reduce((sum, r) => sum + r.count, 0);
  return (
    <div
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: "12px 14px",
      }}
    >
      <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", marginBottom: 8 }}>
        {total} total · peak {max}/day
      </div>
      <div style={{ display: "flex", alignItems: "flex-end", gap: 2, height: 80 }}>
        {rows.map((r) => (
          <div
            key={r.day}
            title={`${r.day}: ${r.count}`}
            style={{
              flex: 1,
              minWidth: 4,
              height: `${Math.max(2, (r.count / max) * 100)}%`,
              background: "var(--accent-fg, #58a6ff)",
              borderRadius: "2px 2px 0 0",
              opacity: 0.85,
            }}
          />
        ))}
      </div>
      <div style={{ display: "flex", justifyContent: "space-between", marginTop: 4, fontSize: 10, color: "var(--text-muted)" }}>
        <span>{rows[0]?.day}</span>
        <span>{rows[rows.length - 1]?.day}</span>
      </div>
    </div>
  );
}

function formatRelative(iso: string): string {
  const ms = Date.now() - Date.parse(iso);
  if (!Number.isFinite(ms) || ms < 0) return iso;
  const m = Math.round(ms / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

// Re-export so test files can stub:
export const DASHBOARD_VARIANTS: DashboardVariant[] = ["planning", "review", "quality", "visits"];
