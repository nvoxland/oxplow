import { useEffect, useMemo, useState } from "react";
import type { EffortAtSnapshot, Snapshot, Stream } from "../api.js";
import {
  getEffort,
  getTaskSummaries,
  listEffortFiles,
  listEffortsOverlappingRange,
  listSnapshots,
} from "../api.js";
import { groupChangesByEffort, type GroupedChanges } from "../snapshot-effort-grouping.js";
import {
  endpointLabel,
  isPureCommitRange,
  previousSnapshotId,
  resolveEffortEndpoints,
  resolveSnapshotEndpoints,
  snapshotRange,
} from "../diffViewModel.js";
import type { DiffEndpoint } from "../tauri-bridge/generated/bindings.js";
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

/**
 * What a diff view renders. Reached three ways, all via `DiffViewPage`:
 *
 * - `snapshot` — the legacy `snapshotRef(N)` drill-in: a prev→N diff of
 *   a single captured snapshot (full function/duplication analysis,
 *   unchanged from the old SnapshotDetailPage).
 * - `effort` — `effortDiffRef(effortId)`: resolves the effort's own
 *   start/end snapshot bracket on load, with the task title + an
 *   "in progress" notice when the effort is still open.
 * - `endpoints` — `endpointDiffRef(start, end)`: an explicit pair of
 *   snapshot/commit/working endpoints diffed via the unified substrate.
 */
export type DiffViewSpec =
  | { mode: "snapshot"; snapshotId: number }
  | { mode: "effort"; effortId: string }
  | { mode: "endpoints"; start: DiffEndpoint | null; end: DiffEndpoint };

export interface DiffViewPageProps {
  stream: Stream | null;
  spec: DiffViewSpec;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
}

export function DiffViewPage(props: DiffViewPageProps) {
  if (props.spec.mode === "snapshot") {
    return <SnapshotDiffBody {...props} snapshotId={props.spec.snapshotId} />;
  }
  return <EndpointDiffBody {...props} spec={props.spec} />;
}

// ---------------------------------------------------------------------------
// Endpoint / effort diff — the reframed "explicit start→end diff" view.
// ---------------------------------------------------------------------------

interface ResolvedDiff {
  start: DiffEndpoint | null;
  end: DiffEndpoint;
  inProgress: boolean;
  taskId: string | null;
}

function EndpointDiffBody({
  stream,
  spec,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: DiffViewPageProps & { spec: Extract<DiffViewSpec, { mode: "effort" | "endpoints" }> }) {
  const [resolved, setResolved] = useState<ResolvedDiff | null>(null);
  const [resolveError, setResolveError] = useState<string | null>(null);

  // Resolve the spec into concrete endpoints. Endpoints mode is already
  // concrete; effort mode fetches the effort's snapshot bracket (so a
  // cold history reopen with only the effort id still works).
  useEffect(() => {
    let cancelled = false;
    if (spec.mode === "endpoints") {
      setResolveError(null);
      setResolved({
        start: spec.start,
        end: spec.end,
        inProgress: spec.end.kind === "working",
        taskId: null,
      });
      return;
    }
    setResolved(null);
    setResolveError(null);
    void getEffort(spec.effortId)
      .then((effort) => {
        if (cancelled) return;
        if (!effort) {
          setResolveError("Effort not found.");
          return;
        }
        const eps = resolveEffortEndpoints(effort);
        setResolved({ ...eps, taskId: effort.taskId });
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "effort resolve failed", { error: String(err) });
        setResolveError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [spec.mode, spec.mode === "effort" ? spec.effortId : "", spec.mode === "endpoints" ? spec.start : null, spec.mode === "endpoints" ? spec.end : null]);

  return (
    <Page testId="page-diff-view" title="Diff" kind="diff-view">
      {resolveError ? (
        <div style={{ ...muted, padding: "12px 16px" }}>{resolveError}</div>
      ) : !resolved ? (
        <div style={{ ...muted, padding: "12px 16px" }}>Loading…</div>
      ) : (
        <ResolvedEndpointDiff
          stream={stream}
          resolved={resolved}
          tabKey={specKey(spec)}
          onOpenPage={onOpenPage}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      )}
    </Page>
  );
}

function specKey(spec: Extract<DiffViewSpec, { mode: "effort" | "endpoints" }>): string {
  return spec.mode === "effort"
    ? `effort:${spec.effortId}`
    : `endpoints:${JSON.stringify(spec.start)}:${JSON.stringify(spec.end)}`;
}

function ResolvedEndpointDiff({
  stream,
  resolved,
  tabKey,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: {
  stream: Stream | null;
  resolved: ResolvedDiff;
  tabKey: string;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
}) {
  const { start, end, inProgress, taskId } = resolved;

  // Snapshot id → its pinned git commit + capture time, for header
  // labels. Cheap window fetch, same pattern the snapshot body uses.
  const [snapshotsById, setSnapshotsById] = useState<Map<number, Snapshot>>(new Map());
  useEffect(() => {
    if (!stream) return;
    let cancelled = false;
    void listSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        setSnapshotsById(new Map(rows.map((r) => [r.id, r])));
      })
      .catch((err) => logUi("warn", "snapshot window fetch failed", { error: String(err) }));
    return () => {
      cancelled = true;
    };
  }, [stream?.id]);

  const snapshotCommits = useMemo(() => {
    const m = new Map<number, string | null>();
    for (const [id, snap] of snapshotsById) m.set(id, snap.gitCommit ?? null);
    return m;
  }, [snapshotsById]);

  // Endpoint-diff analysis. An in-progress effort diffs its start
  // snapshot against the live working tree (the `working` endpoint,
  // supported by the substrate as of tsk339); a small header note flags
  // that the end side is moving.
  const endpoints = useMemo(
    () => ({ start, end }),
    [JSON.stringify(start), JSON.stringify(end)],
  );
  const analysis = useChangeAnalysis({
    streamId: stream?.id ?? null,
    target: tabKey,
    endpoints,
  });

  // Task title for the header link (effort mode).
  const [taskTitle, setTaskTitle] = useState<string | null>(null);
  useEffect(() => {
    if (!taskId) {
      setTaskTitle(null);
      return;
    }
    let cancelled = false;
    void getTaskSummaries([taskId])
      .then((rows) => {
        if (cancelled) return;
        setTaskTitle(rows.find((r) => r.id === taskId)?.title ?? null);
      })
      .catch(() => setTaskTitle(null));
    return () => {
      cancelled = true;
    };
  }, [taskId]);

  // Other efforts whose snapshot window overlaps this range. Hidden for
  // a pure commit↔commit range (no snapshot ids to overlap against).
  const range = useMemo(() => snapshotRange(start, end), [JSON.stringify(start), JSON.stringify(end)]);
  const [effortRows, setEffortRows] = useState<EffortRow[]>([]);
  useEffect(() => {
    if (!range) {
      setEffortRows([]);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const overlapping = await listEffortsOverlappingRange(range.rangeStart, range.rangeEnd);
        if (overlapping.length === 0) {
          if (!cancelled) setEffortRows([]);
          return;
        }
        const titles = await getTaskSummaries(
          Array.from(new Set(overlapping.map((o) => o.taskId))),
        ).catch(() => [] as Array<{ id: string; title: string }>);
        const titleByTask = new Map(titles.map((t) => [t.id, t.title] as const));
        type FileRow = { path: string; change: "created" | "updated" | "deleted" };
        const filesByEffort = await Promise.all(
          overlapping.map(async (o) => {
            try {
              return [o.effortId, await listEffortFiles(o.effortId)] as [string, FileRow[]];
            } catch {
              return [o.effortId, [] as FileRow[]] as [string, FileRow[]];
            }
          }),
        );
        const filesById = new Map(filesByEffort);
        if (cancelled) return;
        setEffortRows(
          overlapping.map((o) => ({
            effort: {
              snapshotId: range.rangeEnd,
              effortId: o.effortId,
              tasksId: o.taskId,
              threadId: o.threadId,
              startSnapshotId: o.startSnapshotId,
              endSnapshotId: o.endSnapshotId,
              completedHere: o.endSnapshotId === range.rangeEnd,
            },
            taskTitle: titleByTask.get(o.taskId) ?? `task ${o.taskId}`,
            files: filesById.get(o.effortId) ?? [],
          })),
        );
      } catch (err) {
        logUi("warn", "overlapping efforts fetch failed", { error: String(err) });
        if (!cancelled) setEffortRows([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [range?.rangeStart, range?.rangeEnd]);

  const effortById = useMemo(
    () => new Map(effortRows.map((r) => [r.effort.effortId, r])),
    [effortRows],
  );
  const grouped = useMemo<GroupedChanges>(
    () =>
      groupChangesByEffort(
        analysis.files.map((f) => ({ path: f.path, status: f.status })),
        effortRows.map((r) => ({ effortId: r.effort.effortId, title: r.taskTitle, files: r.files })),
      ),
    [analysis.files, effortRows],
  );

  const showEffortSection = range !== null && !isPureCommitRange(start, end);
  const startLbl = endpointLabel(start, snapshotCommits);
  const endLbl = endpointLabel(end, snapshotCommits);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: "12px 16px" }}>
      {taskTitle && taskId ? (
        <button
          type="button"
          onClick={() => onOpenPage(taskRef(taskId))}
          style={{ ...linkButton, fontFamily: "inherit", fontWeight: 600, fontSize: "var(--text-sm)" }}
        >
          {taskTitle}
        </button>
      ) : null}

      <DiffHeader
        start={startLbl}
        end={endLbl}
        onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))}
      />

      {inProgress ? (
        <div
          style={{ ...card, color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}
          data-testid="diff-view-in-progress"
        >
          Effort is in progress — diffing the start snapshot against the live
          working tree, which keeps changing until it closes.
        </div>
      ) : null}

      {(
        <>
          {analysis.error ? (
            <div style={{ ...card, color: "var(--severity-critical, #f87171)", fontSize: "var(--text-sm)" }}>
              {analysis.error}
            </div>
          ) : null}

          {analysis.files.length > 0 ? (
            <div style={{ maxWidth: 420 }}>
              <SummaryCard
                fileCount={analysis.files.length}
                additions={analysis.totals.additions}
                deletions={analysis.totals.deletions}
                byStatus={analysis.pivots.byStatus}
                tests={analysis.tests}
                testFunctions={summarizeTestFunctions(analysis.functions)}
                testLineRatio={summarizeTestLineRatio(analysis.functionChurn)}
              />
            </div>
          ) : !analysis.loading && !analysis.error ? (
            <div style={{ ...card, ...muted }}>No file changes between these endpoints.</div>
          ) : null}

          {showEffortSection ? (
            <ChangesByEffortSection
              grouped={grouped}
              effortById={effortById}
              onOpenTask={(id) => onOpenPage(taskRef(id))}
              onOpenSnapshot={(id) => onOpenPage(snapshotRef(id))}
              onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
            />
          ) : analysis.files.length > 0 ? (
            <PlainChangedFiles
              files={analysis.files.map((f) => ({ path: f.path, status: f.status }))}
              onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
            />
          ) : null}

          {/* Full change analysis: zones bar, treemap, and the function /
              churn / duplication drilldown. The endpoint branch now runs
              the same analyzer the snapshot/commit targets do (tsk341),
              so this is the identical panel the legacy snapshot page
              rendered. */}
          {analysis.files.length > 0 && onOpenFile ? (
            <ChangeAnalysisPanel
              analysis={analysis}
              target={tabKey}
              showHeader={false}
              onOpenPage={onOpenPage}
              onOpenFile={onOpenFile}
              onOpenDiff={onOpenDiff}
              onOpenDiffInTab={onOpenDiffInTab}
            />
          ) : null}
        </>
      )}
    </div>
  );
}

/// Diff-oriented header: start → end, each endpoint a clickable commit
/// short-sha when it maps to one, else a plain label.
function DiffHeader({
  start,
  end,
  onOpenCommit,
}: {
  start: ReturnType<typeof endpointLabel>;
  end: ReturnType<typeof endpointLabel>;
  onOpenCommit(sha: string): void;
}) {
  const pill = (lbl: ReturnType<typeof endpointLabel>) =>
    lbl.commitSha ? (
      <button type="button" onClick={() => onOpenCommit(lbl.commitSha!)} style={linkButton}>
        {lbl.text}
      </button>
    ) : (
      <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>{lbl.text}</span>
    );
  return (
    <div
      style={{ ...card, display: "flex", alignItems: "center", gap: 10, fontSize: "var(--text-sm)" }}
      data-testid="diff-view-header"
    >
      {pill(start)}
      <span style={{ color: "var(--text-muted)" }}>→</span>
      {pill(end)}
    </div>
  );
}

/// Flat changed-file list used when there's no effort roster to group
/// by (e.g. a commit↔commit range).
function PlainChangedFiles({
  files,
  onOpenFile,
}: {
  files: Array<{ path: string; status: string }>;
  onOpenFile?: (path: string) => void;
}) {
  return (
    <section style={card}>
      <div style={{ fontWeight: 600, marginBottom: 8, fontSize: "var(--text-sm)" }}>
        {files.length} file{files.length === 1 ? "" : "s"} changed
      </div>
      <ul style={listStyle}>
        {files.map((f) => (
          <li key={f.path} style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
            <StatusBadge status={f.status} />
            {onOpenFile ? (
              <button
                type="button"
                onClick={() => onOpenFile(f.path)}
                style={{ ...linkButton, fontFamily: "var(--mono, monospace)" }}
                title={f.path}
              >
                {f.path}
              </button>
            ) : (
              <span style={{ fontFamily: "var(--mono, monospace)" }}>{f.path}</span>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Snapshot diff (legacy prev→N drill-in). Behavior unchanged from the
// former SnapshotDetailPage.
// ---------------------------------------------------------------------------

interface EffortRow {
  effort: EffortAtSnapshot;
  taskTitle: string;
  files: Array<{ path: string; change: "created" | "updated" | "deleted" }>;
}

function SnapshotDiffBody({
  stream,
  snapshotId,
  onOpenDiff,
  onOpenDiffInTab,
  onOpenPage,
  onOpenFile,
}: DiffViewPageProps & { snapshotId: number }) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [resolved, setResolved] = useState<ResolvedDiff | null>(null);
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
  // A single captured snapshot is framed as the diff [prev → N]: the
  // previous capture in the stream is the start, this snapshot the end.
  // One diff path now (tsk341) — this body resolves the bracket and
  // keeps the snapshot's backlinks, then renders ResolvedEndpointDiff.
  useEffect(() => {
    if (!stream) {
      setSnapshot(null);
      setResolved(null);
      return;
    }
    let cancelled = false;
    // No "get one snapshot" IPC; pull the recent window, pick our row,
    // and find the previous capture. Cheap (~500 rows) for a page load.
    void listSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        setSnapshot(rows.find((r) => r.id === snapshotId) ?? null);
        const prev = previousSnapshotId(
          snapshotId,
          rows.map((r) => r.id),
        );
        setResolved({ ...resolveSnapshotEndpoints(snapshotId, prev), taskId: null });
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot fetch failed", { error: String(err) });
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, snapshotId]);

  const headerTitle = snapshot
    ? `Snapshot ${snapshotId} · ${formatFullDateTime(snapshot.createdAt)}`
    : `Snapshot ${snapshotId}`;

  return (
    <Page testId="page-snapshot-detail" title={headerTitle} kind="snapshot" backlinks={backlinks} outbound={outbound}>
      {!resolved ? (
        <div style={{ ...muted, padding: "12px 16px" }}>Loading…</div>
      ) : (
        <ResolvedEndpointDiff
          stream={stream}
          resolved={resolved}
          tabKey={`snapshot:${snapshotId}`}
          onOpenPage={onOpenPage}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      )}
    </Page>
  );
}

/// The page's core: a snapshot's changed files, grouped by the
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
          ? "Changes in this diff"
          : `${changedCount} file${changedCount === 1 ? "" : "s"} changed`}
      </div>
      {changedCount === 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 11 }}>
          No file changes were captured in this range.
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
              title="Changed in this range but no overlapping effort claimed them — formatters, codegen, parallel actors, or a capture gap."
            >
              Unclaimed
            </div>
            <ul style={listStyle}>{grouped.unclaimed.map((f) => fileRow(f, f.entry.path))}</ul>
          </div>
        ) : null}
      </div>

      {idleRows.length > 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 10, marginTop: 8 }}>
          Also overlapping this range (claimed none of these changes):{" "}
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
        Files are this range's actual diff, attributed to the effort(s) whose declared
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
