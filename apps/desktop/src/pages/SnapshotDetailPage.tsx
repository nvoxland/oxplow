import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { EffortAtSnapshot, Snapshot, Stream } from "../api.js";
import {
  getTaskSummaries,
  listEffortFiles,
  listEffortsAtSnapshots,
  listSnapshots,
} from "../api.js";
import { groupChangesByEffort, type GroupedChanges } from "../snapshot-effort-grouping.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { ChangeAnalysisPanel } from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import {
  summarizeTestFunctions,
  summarizeTestLineRatio,
} from "../components/ChangeAnalysis/analysisHelpers.js";
import { formatFullDateTime } from "../components/format.js";

export interface SnapshotDetailPageProps {
  stream: Stream | null;
  snapshotId: number;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Single snapshot page — drilled into from the Local History
 * dashboard's Recent Snapshots card. Mirrors GitCommitPage: a
 * SummaryCard on the right, snapshot metadata on the left, and the
 * shared ChangeAnalysisPanel below for the file list + function /
 * churn / interestingness breakdowns.
 *
 * The change-analysis pipeline is fed via a `target` string of
 * `"snapshot:<id>"`; useChangeAnalysis recognizes that and routes
 * file lookups + content reads through the snapshot store instead
 * of git refs.
 */
interface EffortRow {
  effort: EffortAtSnapshot;
  taskTitle: string;
  files: Array<{ path: string; change: "created" | "updated" | "deleted" }>;
}

export function SnapshotDetailPage({
  stream,
  snapshotId,
  onOpenDiff,
  onOpenDiffInTab,
  onOpenPage,
  onOpenFile,
}: SnapshotDetailPageProps) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loadingSnapshot, setLoadingSnapshot] = useState(false);
  const [completedEfforts, setCompletedEfforts] = useState<EffortRow[]>([]);
  const [inFlightEfforts, setInFlightEfforts] = useState<EffortRow[]>([]);
  const refForGraph = snapshotRef(snapshotId);
  const backlinkEntries = useBacklinks(refForGraph);
  const outboundEntries = usePageOutbound(refForGraph);
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
  const target = `snapshot:${snapshotId}`;
  const analysis = useChangeAnalysis({ streamId: stream?.id ?? null, target });

  // Measure the SummaryCard so the SnapshotMeta on its left collapses
  // to the same height — same pattern GitCommitPage uses.
  const summaryRef = useRef<HTMLDivElement>(null);
  const [summaryHeight, setSummaryHeight] = useState<number | null>(null);
  useLayoutEffect(() => {
    const el = summaryRef.current;
    if (!el) {
      setSummaryHeight(null);
      return;
    }
    const measure = () => setSummaryHeight(el.getBoundingClientRect().height);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [analysis.files.length, snapshotId, stream?.id]);

  useEffect(() => {
    if (!stream) {
      setSnapshot(null);
      return;
    }
    let cancelled = false;
    setLoadingSnapshot(true);
    // No "get one snapshot" IPC; pull the recent window and pick our id.
    // Cheap (~500 rows) — acceptable for a page-load fetch.
    void listSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        setSnapshot(rows.find((r) => r.id === snapshotId) ?? null);
        setLoadingSnapshot(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot fetch failed", { error: String(err) });
        setLoadingSnapshot(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, snapshotId]);

  // Efforts active at this snapshot, partitioned into "completed
  // here" (end_snapshot_id == this snapshot — the "this is what
  // shipped" view) and "in flight" (start_snapshot_id <= this AND
  // end is later or NULL). Each row carries the canonical
  // task_effort_file list — the LLM-declared authorship (after
  // any amend_effort), not the raw snapshot diff. The auto-diff
  // is only fetched for completed efforts (it requires both start
  // and end snapshot ids).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const all = await listEffortsAtSnapshots([snapshotId]);
        if (all.length === 0) {
          if (!cancelled) {
            setCompletedEfforts([]);
            setInFlightEfforts([]);
          }
          return;
        }
        const completed = all.filter((e) => e.completedHere);
        const inFlight = all.filter((e) => !e.completedHere);
        const titles = await getTaskSummaries(
          Array.from(new Set(all.map((e) => e.tasksId))),
        ).catch(() => [] as Array<{ id: string; title: string }>);
        const titleByTask = new Map<string, string>(
          titles.map((t) => [t.id, t.title] as [string, string]),
        );
        type EffortFileRow = { path: string; change: "created" | "updated" | "deleted" };
        const filesByEffort: Array<[string, EffortFileRow[]]> = await Promise.all(
          all.map(async (e) => {
            try {
              const files = await listEffortFiles(e.effortId);
              return [e.effortId, files] as [string, EffortFileRow[]];
            } catch (err) {
              logUi("warn", "effort files fetch failed", {
                error: String(err),
                effortId: e.effortId,
              });
              return [e.effortId, [] as EffortFileRow[]] as [string, EffortFileRow[]];
            }
          }),
        );
        const filesById = new Map<string, EffortFileRow[]>(filesByEffort);
        if (cancelled) return;
        const toRow = (effort: EffortAtSnapshot): EffortRow => ({
          effort,
          taskTitle: titleByTask.get(effort.tasksId) ?? `task ${effort.tasksId}`,
          files: filesById.get(effort.effortId) ?? [],
        });
        setCompletedEfforts(completed.map(toRow));
        setInFlightEfforts(inFlight.map(toRow));
      } catch (err) {
        logUi("warn", "efforts at snapshot fetch failed", { error: String(err) });
        if (!cancelled) {
          setCompletedEfforts([]);
          setInFlightEfforts([]);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [snapshotId]);

  const headerTitle = useMemo(() => {
    if (!snapshot) return `Snapshot ${snapshotId}`;
    return `Snapshot ${snapshotId} · ${formatFullDateTime(snapshot.createdAt)}`;
  }, [snapshot, snapshotId]);

  // The page's core: this snapshot's actual changed files, grouped by
  // the effort(s) that claim them. All efforts active here (completed +
  // in-flight) are candidate claimers; the helper buckets the rest into
  // "unclaimed" and reports efforts that claimed nothing changed here.
  const allEffortRows = useMemo(
    () => [...completedEfforts, ...inFlightEfforts],
    [completedEfforts, inFlightEfforts],
  );
  const effortById = useMemo(
    () => new Map(allEffortRows.map((r) => [r.effort.effortId, r])),
    [allEffortRows],
  );
  const grouped = useMemo<GroupedChanges>(
    () =>
      groupChangesByEffort(
        analysis.files.map((f) => ({ path: f.path, status: f.status })),
        allEffortRows.map((r) => ({
          effortId: r.effort.effortId,
          title: r.taskTitle,
          files: r.files,
        })),
      ),
    [analysis.files, allEffortRows],
  );

  return (
    <Page testId="page-snapshot-detail" title={headerTitle} kind="snapshot" backlinks={backlinks} outbound={outbound}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: "12px 16px" }}>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(0, 3fr) minmax(0, 2fr)",
            gap: 16,
            alignItems: "start",
          }}
        >
          <div style={{ minWidth: 0 }}>
            {loadingSnapshot && !snapshot ? (
              <div style={muted}>Loading…</div>
            ) : !snapshot ? (
              <div style={muted}>Snapshot not found.</div>
            ) : (
              <SnapshotMeta
                snapshot={snapshot}
                collapsedMaxHeight={summaryHeight}
                onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))}
              />
            )}
          </div>
          <div style={{ minWidth: 0 }} ref={summaryRef}>
            {snapshot && stream && analysis.files.length > 0 ? (
              <SummaryCard
                fileCount={analysis.files.length}
                additions={analysis.totals.additions}
                deletions={analysis.totals.deletions}
                byStatus={analysis.pivots.byStatus}
                tests={analysis.tests}
                testFunctions={summarizeTestFunctions(analysis.functions)}
                testLineRatio={summarizeTestLineRatio(analysis.functionChurn)}
              />
            ) : null}
          </div>
        </div>

        {snapshot && stream ? (
          <ChangesByEffortSection
            grouped={grouped}
            effortById={effortById}
            onOpenTask={(taskId) => onOpenPage(taskRef(taskId))}
            onOpenSnapshot={(id) => onOpenPage(snapshotRef(id))}
            onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
          />
        ) : null}

        {snapshot && stream && onOpenFile ? (
          <ChangeAnalysisPanel
            analysis={analysis}
            target={target}
            showHeader={false}
            onOpenPage={onOpenPage}
            onOpenFile={onOpenFile}
            onOpenDiff={onOpenDiff}
            onOpenDiffInTab={onOpenDiffInTab}
          />
        ) : null}
      </div>
    </Page>
  );
}

/// The page's core: this snapshot's changed files, grouped by the
/// effort(s) that claim them, then an "Unclaimed" bucket, then a roster
/// of efforts active here that claimed none of the changes.
function ChangesByEffortSection({
  grouped,
  effortById,
  onOpenTask,
  onOpenSnapshot,
  onOpenFile,
}: {
  grouped: GroupedChanges;
  effortById: Map<string, EffortRow>;
  onOpenTask(taskId: string): void;
  onOpenSnapshot(snapshotId: number): void;
  onOpenFile?: (path: string) => void;
}) {
  const changedCount =
    grouped.byEffort.reduce((n, g) => n + g.files.length, 0) + grouped.unclaimed.length;
  const idleRows = grouped.idleEffortIds
    .map((id) => effortById.get(id))
    .filter((r): r is EffortRow => !!r);

  const fileRow = (
    f: GroupedChanges["unclaimed"][number],
    key: string,
  ) => (
    <li key={key} style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
      <StatusBadge status={f.entry.status} />
      {onOpenFile ? (
        <button
          type="button"
          onClick={() => onOpenFile(f.entry.path)}
          style={{ ...linkButton, fontFamily: "var(--mono, monospace)" }}
          title={f.entry.path}
        >
          {f.entry.path}
        </button>
      ) : (
        <span style={{ fontFamily: "var(--mono, monospace)" }}>{f.entry.path}</span>
      )}
      {f.declaredChange ? (
        <span style={{ color: "var(--text-muted)", fontSize: 10 }}>claimed: {f.declaredChange}</span>
      ) : null}
      {f.alsoClaimedBy.length > 0 ? (
        <span style={{ color: "var(--text-muted)", fontSize: 10 }}>
          · also claimed by {f.alsoClaimedBy.join(", ")}
        </span>
      ) : null}
    </li>
  );

  return (
    <section style={card}>
      <div style={{ fontWeight: 600, marginBottom: 8, fontSize: "var(--text-sm)" }}>
        {changedCount === 0
          ? "Changes in this snapshot"
          : `${changedCount} file${changedCount === 1 ? "" : "s"} changed in this snapshot`}
      </div>
      {changedCount === 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 11 }}>
          No file changes were captured in this snapshot.
        </div>
      ) : null}
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        {grouped.byEffort.map((group) => {
          const row = effortById.get(group.effortId);
          const effort = row?.effort;
          return (
            <div key={group.effortId} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                <button
                  type="button"
                  onClick={() => effort && onOpenTask(effort.tasksId)}
                  style={{ ...linkButton, fontFamily: "inherit", fontWeight: 600 }}
                >
                  {group.title}
                </button>
                {effort ? (
                  <span style={{ color: "var(--text-secondary)", fontSize: 10 }}>
                    {effort.completedHere ? "completed here" : "in progress"}
                  </span>
                ) : null}
                {effort?.startSnapshotId != null && !effort.completedHere ? (
                  <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>
                    · started at{" "}
                    <button
                      type="button"
                      onClick={() => onOpenSnapshot(effort.startSnapshotId!)}
                      style={linkButton}
                    >
                      snapshot {effort.startSnapshotId}
                    </button>
                  </span>
                ) : null}
              </div>
              <ul style={listStyle}>{group.files.map((f) => fileRow(f, f.entry.path))}</ul>
            </div>
          );
        })}

        {grouped.unclaimed.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <div
              style={{ fontWeight: 600, fontSize: 11, color: "var(--freshness-stale)" }}
              title="Changed in this snapshot but no active effort claimed them — formatters, codegen, parallel actors, or a capture gap."
            >
              Unclaimed
            </div>
            <ul style={listStyle}>{grouped.unclaimed.map((f) => fileRow(f, f.entry.path))}</ul>
          </div>
        ) : null}
      </div>

      {idleRows.length > 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 10, marginTop: 8 }}>
          Also active here (claimed no changes in this snapshot):{" "}
          {idleRows.map((r, i) => (
            <span key={r.effort.effortId}>
              {i > 0 ? ", " : ""}
              <button type="button" onClick={() => onOpenTask(r.effort.tasksId)} style={linkButton}>
                {r.taskTitle}
              </button>
            </span>
          ))}
        </div>
      ) : null}

      <div style={{ color: "var(--text-muted)", fontSize: 10, marginTop: 8 }}>
        Files are this snapshot's actual diff, attributed to the effort(s) whose declared
        authorship (via <code>complete_task</code>/<code>amend_effort</code>) includes them.
      </div>
    </section>
  );
}

/// Small colored chip for a snapshot file status.
function StatusBadge({ status }: { status: string }) {
  const color =
    status === "added"
      ? "var(--status-done, #4caf50)"
      : status === "deleted"
        ? "var(--severity-critical, #f87171)"
        : "var(--text-secondary)";
  const label = status === "added" ? "A" : status === "deleted" ? "D" : status === "modified" ? "M" : status[0]?.toUpperCase() ?? "?";
  return (
    <span
      title={status}
      style={{
        fontFamily: "var(--mono, monospace)",
        fontSize: 10,
        fontWeight: 700,
        color,
        minWidth: 12,
        textAlign: "center",
      }}
    >
      {label}
    </span>
  );
}


const listStyle: React.CSSProperties = {
  margin: 0,
  paddingLeft: 18,
  fontSize: 11,
  color: "var(--text-secondary)",
  listStyle: "none",
  display: "flex",
  flexDirection: "column",
  gap: 2,
};

interface SnapshotMetaProps {
  snapshot: Snapshot;
  /** Pixel height the SummaryCard alongside this panel renders at —
   *  the panel collapses to roughly that height. `null` lets it grow. */
  collapsedMaxHeight: number | null;
  onOpenCommit(sha: string): void;
}

function SnapshotMeta({ snapshot, collapsedMaxHeight, onOpenCommit }: SnapshotMetaProps) {
  const captured = formatFullDateTime(snapshot.createdAt);
  return (
    <section
      style={{
        ...card,
        maxHeight: collapsedMaxHeight ?? undefined,
        overflow: collapsedMaxHeight ? "hidden" : undefined,
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--text-xs)" }}>
        <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
          Captured {captured}
        </div>
        <div>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>
            Snapshot {snapshot.id}
          </div>
          <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
            {snapshot.fileCount} files captured in this snapshot.
          </div>
        </div>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "auto 1fr",
            gap: "2px 10px",
            color: "var(--text-secondary)",
            fontSize: 11,
          }}
        >
          <span>Git commit</span>
          <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>
            {snapshot.gitCommit ? (
              <button
                type="button"
                onClick={() => onOpenCommit(snapshot.gitCommit!)}
                style={linkButton}
              >
                {snapshot.gitCommit.slice(0, 7)}
              </button>
            ) : (
              <span style={{ color: "var(--text-secondary)" }}>—</span>
            )}
          </span>
        </div>
      </div>
    </section>
  );
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  position: "relative",
};
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontFamily: "var(--mono, monospace)",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};
