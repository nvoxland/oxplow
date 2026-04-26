import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { BacklogState, CodeQualityFindingRow, FileSnapshot, Stream, ThreadWorkState, WikiNoteSummary, WorkItem } from "../api.js";
import {
  listCodeQualityFindings,
  listSnapshots,
  listWikiNotes,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { findingRef, indexRef, noteRef, workItemRef } from "../tabs/pageRefs.js";

export type DashboardVariant = "planning" | "review" | "quality";

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
};

/**
 * Composite dashboard pages — Planning, Review, Quality. Each is a
 * read-only summary stitched together from existing data slices: no new
 * IPC, no new mutations, just buttons that route through `onOpenPage`.
 */
export function DashboardPage({ variant, stream, threadWork, backlog, onOpenPage }: DashboardPageProps) {
  return (
    <Page testId={`page-dashboard-${variant}`} title={VARIANT_TITLE[variant]} kind="dashboard">
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

function RowButton({ label, subtitle, onClick, testId }: { label: string; subtitle?: string; onClick(): void; testId?: string }) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
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
        fontSize: 13,
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
    <div style={{ color: "var(--text-secondary)", fontSize: 12, fontStyle: "italic" }}>{children}</div>
  );
}

function useRecentNotes(stream: Stream | null) {
  const [notes, setNotes] = useState<WikiNoteSummary[]>([]);
  useEffect(() => {
    if (!stream) {
      setNotes([]);
      return;
    }
    let cancelled = false;
    void listWikiNotes(stream.id).then((rows) => {
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
    void listSnapshots(stream.id).then((rows) => {
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
  const ready: WorkItem[] = threadWork?.waiting ?? [];
  const backlogItems = backlog?.items ?? [];
  const notes = useRecentNotes(stream);

  return (
    <>
      <Section title="Ready in this thread">
        {ready.length === 0 ? <EmptyHint>Nothing ready.</EmptyHint> : null}
        {ready.slice(0, 10).map((item) => (
          <RowButton
            key={item.id}
            testId={`dashboard-planning-ready-${item.id}`}
            label={item.title}
            subtitle={item.priority}
            onClick={() => onOpenPage(workItemRef(item.id))}
          />
        ))}
      </Section>
      <Section title="Backlog">
        {backlogItems.length === 0 ? <EmptyHint>Backlog is empty.</EmptyHint> : null}
        {backlogItems.slice(0, 10).map((item) => (
          <RowButton
            key={item.id}
            label={item.title}
            subtitle={item.priority}
            onClick={() => onOpenPage(workItemRef(item.id))}
          />
        ))}
      </Section>
      <Section title="Recent notes">
        {notes.length === 0 ? <EmptyHint>No notes yet.</EmptyHint> : null}
        {notes.slice(0, 8).map((note) => (
          <RowButton
            key={note.slug}
            label={note.title || note.slug}
            subtitle={note.freshness}
            onClick={() => onOpenPage(noteRef(note.slug))}
          />
        ))}
      </Section>
      <Section title="Subsystem docs">
        <RowButton
          label="Open subsystem docs index"
          onClick={() => onOpenPage(indexRef("subsystem-docs"))}
        />
      </Section>
    </>
  );
}

function ReviewSections({
  threadWork,
  stream,
  onOpenPage,
}: {
  threadWork: ThreadWorkState | null;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const humanCheck = useMemo(() => {
    const items = threadWork?.items ?? [];
    return items.filter((i) => i.status === "human_check");
  }, [threadWork]);
  const snaps = useRecentSnapshots(stream);
  const findings = useFindings(stream);

  return (
    <>
      <Section title="Awaiting human check">
        {humanCheck.length === 0 ? <EmptyHint>Nothing waiting on review.</EmptyHint> : null}
        {humanCheck.map((item) => (
          <RowButton
            key={item.id}
            testId={`dashboard-review-hc-${item.id}`}
            label={item.title}
            subtitle={item.priority}
            onClick={() => onOpenPage(workItemRef(item.id))}
          />
        ))}
      </Section>
      <Section title="Recent snapshots">
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
      <Section title="New findings">
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
      <Section title="All findings">
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
      <Section title="Complexity outliers">
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
      <Section title="Duplicate blocks">
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

// Re-export so test files can stub:
export const DASHBOARD_VARIANTS: DashboardVariant[] = ["planning", "review", "quality"];
