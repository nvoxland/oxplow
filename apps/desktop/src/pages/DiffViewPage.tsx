import { useEffect, useMemo, useState } from "react";
import type { BranchChangeEntry, EffortAtSnapshot, EffortObservation, Snapshot, Stream } from "../api.js";
import {
  getEffort,
  getTaskSummaries,
  listEffortFiles,
  listEffortObservations,
  listEffortsOverlappingRange,
  listSnapshots,
  subscribeOxplowEvents,
} from "../api.js";
import {
  previousSnapshotId,
  resolveEffortEndpoints,
  resolveSnapshotEndpoints,
  snapshotRange,
} from "../diffViewModel.js";
import type { DiffEndpoint } from "../tauri-bridge/generated/bindings.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { DISK, refVersion } from "../file-version.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import {
  effortCoverageRef,
  effortDiffRef,
  endpointDiffRef,
  gitCommitRef,
  snapshotRef,
  taskRef,
} from "../tabs/pageRefs.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { ChangeAnalysisPanel } from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import { ChangeAnalysisFileTree } from "../components/ChangeAnalysis/FileTreeView.js";
import { ChangeTreemapCard } from "../components/ChangeAnalysis/ChangeTreemapCard.js";
import { LookHereFirstCard } from "../components/ChangeAnalysis/LookHereFirstCard.js";
import { FunctionsCard } from "../components/ChangeAnalysis/FunctionsCard.js";
import type { FunctionsBuckets } from "../components/ChangeAnalysis/analysisHelpers.js";
import { TestsRun } from "../components/EffortObservations.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import { isTestPath } from "../components/ChangeAnalysis/analysisHelpers.js";
import { formatFullDateTime, formatTimeOnly, isSameCalendarDay } from "../components/format.js";

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
  return <DiffBody {...props} />;
}

// ---------------------------------------------------------------------------
// Endpoint / effort diff — the reframed "explicit start→end diff" view.
// ---------------------------------------------------------------------------

interface ResolvedDiff {
  start: DiffEndpoint | null;
  end: DiffEndpoint;
  inProgress: boolean;
  /** Task id of the effort this diff was opened *for* (effort mode);
   *  null for snapshot/endpoint diffs. */
  taskId: string | null;
  /** Effort id when the diff was opened *for* an effort (effort mode).
   *  Drives the claimed-files filter; null otherwise. */
  effortId: string | null;
}

/**
 * The one diff body. Resolves any of the three specs to concrete
 * endpoints and renders the shared `ResolvedEndpointDiff`:
 * - **endpoints** — already concrete.
 * - **effort** — fetches the effort's start/end bracket (survives a
 *   cold reopen with only the effort id).
 * - **snapshot** — a single capture framed as `[prev → N]`; resolves the
 *   previous capture from the stream's snapshot list.
 */
function DiffBody({
  stream,
  spec,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: DiffViewPageProps) {
  const [resolved, setResolved] = useState<ResolvedDiff | null>(null);
  const [resolveError, setResolveError] = useState<string | null>(null);
  const key = specKey(spec);

  // Backlinks/outbound keyed on the ref that opened this page. A
  // snapshot is a linkable entity; effort/endpoint diffs rarely are,
  // but a stable identity keeps the page chrome uniform.
  const graphRef = useMemo<TabRef>(() => {
    if (spec.mode === "snapshot") return snapshotRef(spec.snapshotId);
    if (spec.mode === "effort") return effortDiffRef(spec.effortId);
    return endpointDiffRef(spec.start, spec.end);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const backlinkEntries = useBacklinks(graphRef);
  const outboundEntries = usePageOutbound(graphRef);
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

  useEffect(() => {
    let cancelled = false;
    setResolveError(null);
    if (spec.mode === "endpoints") {
      setResolved({
        start: spec.start,
        end: spec.end,
        inProgress: spec.end.kind === "working",
        taskId: null,
        effortId: null,
      });
      return;
    }
    setResolved(null);
    if (spec.mode === "effort") {
      void getEffort(spec.effortId)
        .then((effort) => {
          if (cancelled) return;
          if (!effort) {
            setResolveError("Effort not found.");
            return;
          }
          setResolved({
            ...resolveEffortEndpoints(effort),
            taskId: effort.taskId,
            effortId: effort.effortId,
          });
        })
        .catch((err) => {
          if (cancelled) return;
          logUi("warn", "effort resolve failed", { error: String(err) });
          setResolveError(err instanceof Error ? err.message : String(err));
        });
      return () => {
        cancelled = true;
      };
    }
    // Snapshot mode: resolve [prev → N] from the stream's capture list.
    if (!stream) {
      setResolved(null);
      return;
    }
    const snapshotId = spec.snapshotId;
    void listSnapshots(stream.id, 500)
      .then((rows) => {
        if (cancelled) return;
        const prev = previousSnapshotId(
          snapshotId,
          rows.map((r) => r.id),
        );
        setResolved({
          ...resolveSnapshotEndpoints(snapshotId, prev),
          taskId: null,
          effortId: null,
        });
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot fetch failed", { error: String(err) });
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stream?.id, key]);

  return (
    <Page
      testId="page-diff-view"
      title={resolved ? undefined : "Changes"}
      kind="diff-view"
      backlinks={backlinks}
      outbound={outbound}
    >
      {resolveError ? (
        <div style={{ ...muted, padding: "12px 16px" }}>{resolveError}</div>
      ) : !resolved ? (
        <div style={{ ...muted, padding: "12px 16px" }}>Loading…</div>
      ) : (
        <ResolvedEndpointDiff
          stream={stream}
          resolved={resolved}
          tabKey={key}
          onOpenPage={onOpenPage}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      )}
    </Page>
  );
}

function specKey(spec: DiffViewSpec): string {
  switch (spec.mode) {
    case "snapshot":
      return `snapshot:${spec.snapshotId}`;
    case "effort":
      return `effort:${spec.effortId}`;
    case "endpoints":
      return `endpoints:${JSON.stringify(spec.start)}:${JSON.stringify(spec.end)}`;
  }
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
  const { start, end, inProgress, taskId, effortId } = resolved;
  const effortPassed = effortId != null;

  // Snapshot id → its capture time + pinned git commit, for the title's
  // start/end labels. Cheap window fetch, same pattern the old body used.
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

  // Endpoint-diff analysis. An in-progress effort diffs its start
  // snapshot against the live working tree (the `working` endpoint); a
  // small header note flags that the end side is moving.
  const endpoints = useMemo(
    () => ({ start, end }),
    [JSON.stringify(start), JSON.stringify(end)],
  );
  const analysis = useChangeAnalysis({
    streamId: stream?.id ?? null,
    target: tabKey,
    endpoints,
  });

  // Task title for the header (effort mode).
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

  // Efforts whose snapshot window overlaps this range. Drives both the
  // "Concurrent Efforts" list and the lined-up-effort title detection.
  // Null for a range with no snapshot endpoints to overlap against.
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
            endedAt: o.endedAt,
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

  // Files this effort CLAIMED — only fetched when a diff was opened *for*
  // an effort, where the Files Changed list is restricted to them.
  const [claimedPaths, setClaimedPaths] = useState<Set<string> | null>(null);
  useEffect(() => {
    if (!effortPassed || !effortId) {
      setClaimedPaths(null);
      return;
    }
    let cancelled = false;
    void listEffortFiles(effortId)
      .then((rows) => {
        if (cancelled) return;
        setClaimedPaths(new Set(rows.map((r) => r.path)));
      })
      .catch(() => {
        if (!cancelled) setClaimedPaths(new Set());
      });
    return () => {
      cancelled = true;
    };
  }, [effortPassed, effortId]);

  // Effort identity for the title + concurrent-effort exclusion. The diff
  // is "for an effort" when one was passed (effort mode) OR when the
  // endpoints line up exactly with an overlapping effort's bracket.
  const startSnapId = start?.kind === "snapshot" ? start.snapshot_id : null;
  const endSnapId = end.kind === "snapshot" ? end.snapshot_id : null;
  // Capture time of the range's start snapshot — used to drop efforts that
  // ended before this range began from the concurrent list.
  const rangeStartIso =
    startSnapId != null ? snapshotsById.get(startSnapId)?.createdAt ?? null : null;
  const linedUpEffort = useMemo(() => {
    if (effortPassed || endSnapId == null) return null;
    return (
      effortRows.find(
        (r) => r.effort.startSnapshotId === startSnapId && r.effort.endSnapshotId === endSnapId,
      ) ?? null
    );
  }, [effortPassed, effortRows, startSnapId, endSnapId]);
  const primaryEffortId = effortPassed ? effortId : linedUpEffort?.effort.effortId ?? null;
  const effortTitle = effortPassed ? taskTitle : linedUpEffort?.taskTitle ?? null;

  // Concurrent efforts = every overlapping effort other than the one this
  // diff is for. Drops efforts that had already ENDED before this range began —
  // they surface only because they never pinned an end snapshot (the overlap
  // query treats `end_snapshot_id IS NULL` as still-open), not because they
  // actually ran concurrently.
  const concurrentEfforts = useMemo(
    () =>
      effortRows.filter((r) => {
        if (r.effort.effortId === primaryEffortId) return false;
        if (r.endedAt && rangeStartIso && r.endedAt < rangeStartIso) return false;
        return true;
      }),
    [effortRows, primaryEffortId, rangeStartIso],
  );

  const startDisp = useMemo(
    () => endpointDisplay(start, snapshotsById),
    [JSON.stringify(start), snapshotsById],
  );
  // When the end falls on the same calendar day as the start, collapse it
  // to a time-only label so the date isn't repeated in the range.
  const endDisp = useMemo(() => {
    const disp = endpointDisplay(end, snapshotsById);
    if (disp.iso && startDisp.iso && isSameCalendarDay(startDisp.iso, disp.iso)) {
      return { ...disp, timeText: formatTimeOnly(disp.iso) };
    }
    return disp;
  }, [JSON.stringify(end), snapshotsById, startDisp]);

  // Chrome / tab title — plain-text mirror of the h1.
  const plainTitle = effortTitle
    ? `Changes: ${effortTitle}`
    : `Changes: ${endpointPlain(startDisp)} – ${endpointPlain(endDisp)}`;
  usePageTitle(plainTitle);

  // Files for the Files Changed tree. All changed files by default; only
  // the effort's claimed files when a diff was opened *for* an effort.
  const filesForList = useMemo<BranchChangeEntry[]>(() => {
    if (!effortPassed) return analysis.files;
    if (!claimedPaths) return [];
    return analysis.files.filter((f) => claimedPaths.has(f.path));
  }, [effortPassed, claimedPaths, analysis.files]);
  const filesLoading = analysis.loading || (effortPassed && claimedPaths === null);

  // Function-level changes for the standalone "Function Changes" section,
  // scoped to the effort's claimed files when opened for one (mirrors
  // filesForList). The bottom FilesPanel (which used to host this) is hidden
  // on the diff view.
  const functionsForList = useMemo<FunctionsBuckets>(() => {
    if (!effortPassed || !claimedPaths) return analysis.functions;
    const claimed = claimedPaths;
    const keep = <T extends { path: string }>(rows: T[]): T[] =>
      rows.filter((f) => claimed.has(f.path));
    return {
      added: keep(analysis.functions.added),
      deleted: keep(analysis.functions.deleted),
      modifiedSignature: keep(analysis.functions.modifiedSignature),
      modifiedBody: keep(analysis.functions.modifiedBody),
    };
  }, [effortPassed, claimedPaths, analysis.functions]);
  const hasFunctionChanges =
    functionsForList.added.length +
      functionsForList.deleted.length +
      functionsForList.modifiedSignature.length +
      functionsForList.modifiedBody.length >
    0;

  // Test work in the range — file-based (every test file counts, even
  // ones with only `describe`/`test` blocks and no named functions, which
  // the old function-based summary missed). Scoped to the full range, not
  // the effort's claimed files.
  const testFiles = useMemo(
    () => analysis.files.filter((f) => isTestPath(f.path)),
    [analysis.files],
  );
  const testStats = useMemo(() => {
    let testLines = 0;
    let productionLines = 0;
    for (const f of analysis.files) {
      const lines = (f.additions ?? 0) + (f.deletions ?? 0);
      if (isTestPath(f.path)) testLines += lines;
      else productionLines += lines;
    }
    return {
      testLines,
      productionLines,
      ratio: productionLines > 0 ? testLines / productionLines : 0,
    };
  }, [analysis.files]);

  // Test RUNS recorded during the range — the times tests were executed,
  // unioned across the effort(s) overlapping it (the primary effort in
  // effort mode, plus any concurrent ones). Independent of whether test
  // files changed: tests can be run without editing them.
  const runEffortIds = useMemo(() => {
    const ids = new Set<string>();
    if (effortId) ids.add(effortId);
    for (const r of effortRows) ids.add(r.effort.effortId);
    return [...ids];
  }, [effortId, effortRows]);
  const runEffortKey = runEffortIds.join(",");
  const [testRuns, setTestRuns] = useState<EffortObservation[]>([]);
  useEffect(() => {
    if (runEffortIds.length === 0) {
      setTestRuns([]);
      return;
    }
    let cancelled = false;
    const load = () => {
      void Promise.all(
        runEffortIds.map((id) =>
          listEffortObservations(id, "test-run").catch(() => [] as EffortObservation[]),
        ),
      ).then((lists) => {
        if (!cancelled) setTestRuns(lists.flat());
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind === "effortObservationsChanged") load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runEffortKey]);

  // Open a file's diff in the current tab (revealing `line`), mirroring the
  // drilldown. Used by both the Files tree and the Functions rows.
  const openDiffAt = (path: string, line = 1) => {
    if (!analysis.refs) {
      onOpenFile?.(path);
      return;
    }
    const { baseRef, headRef } = analysis.refs;
    const spec: DiffSpec = {
      path,
      leftVersion: refVersion(baseRef),
      rightVersion: headRef ? refVersion(headRef) : DISK,
      baseLabel: endpointPlain(startDisp),
      revealLine: line,
    };
    if (onOpenDiffInTab) onOpenDiffInTab(spec);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile?.(path);
  };

  const openCommit = (sha: string) => onOpenPage(gitCommitRef(sha));

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: "12px 16px" }}>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <h1 style={h1Style} data-testid="diff-view-title">
          {effortTitle ? (
            `Changes: ${effortTitle}`
          ) : (
            <>
              Changes: <RangeLabel start={startDisp} end={endDisp} onOpenCommit={openCommit} />
            </>
          )}
        </h1>
        {effortTitle ? (
          <div style={subtitleStyle} data-testid="diff-view-subtitle">
            <RangeLabel start={startDisp} end={endDisp} onOpenCommit={openCommit} />
          </div>
        ) : null}
      </div>

      {inProgress ? (
        <div
          style={{ ...card, color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}
          data-testid="diff-view-in-progress"
        >
          Effort is in progress — diffing the start snapshot against the live
          working tree, which keeps changing until it closes.
        </div>
      ) : null}

      {analysis.error ? (
        <div style={{ ...card, color: "var(--severity-critical, #f87171)", fontSize: "var(--text-sm)" }}>
          {analysis.error}
        </div>
      ) : null}

      {concurrentEfforts.length > 0 ? (
        <section data-testid="diff-view-concurrent-efforts">
          <h2 style={h2Style}>Concurrent Efforts</h2>
          <ul style={effortListStyle}>
            {concurrentEfforts.map((r) => (
              <li key={r.effort.effortId}>
                <button
                  type="button"
                  onClick={() => onOpenPage(taskRef(r.effort.tasksId))}
                  style={{ ...linkButton, fontFamily: "inherit", fontSize: "var(--text-sm)" }}
                >
                  {r.taskTitle}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {analysis.files.length > 0 && onOpenFile ? (
        <ChangeTreemapCard
          files={analysis.files}
          functionChurn={analysis.functionChurn}
          onOpenFile={onOpenFile}
          onOpenFileDiff={(path, line) => openDiffAt(path, line ?? 1)}
        />
      ) : null}

      {analysis.files.length > 0 ? (
        <LookHereFirstCard
          boxless
          files={analysis.files}
          fileScores={analysis.fileScores}
          onOpenFile={onOpenFile}
          onOpenFileDiff={(path) => openDiffAt(path, 1)}
        />
      ) : null}

      <section data-testid="diff-view-files-changed">
        <h2 style={h2Style}>Files Changed</h2>
        {filesLoading && filesForList.length === 0 ? (
          <div style={muted}>Loading…</div>
        ) : filesForList.length === 0 ? (
          <div style={muted}>
            {effortPassed
              ? "This effort claimed no changed files."
              : "No file changes between these endpoints."}
          </div>
        ) : onOpenFile ? (
          <ChangeAnalysisFileTree
            files={filesForList}
            target={tabKey}
            onOpenFile={(path, opts) => onOpenFile(path, opts)}
            onOpenFileDiff={(path) => openDiffAt(path, 1)}
            showFileCount={false}
          />
        ) : null}
      </section>

      <section data-testid="diff-view-tests">
        <h2 style={h2Style}>Tests</h2>
        {filesLoading && analysis.files.length === 0 ? (
          <div style={muted}>Loading…</div>
        ) : testFiles.length === 0 && testRuns.length === 0 ? (
          <div style={muted}>No test changes or test runs in this range.</div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            {testRuns.length > 0 ? (
              <TestsRun effortId={effortPassed && effortId ? effortId : undefined} runs={testRuns} />
            ) : (
              <div style={muted}>No test runs recorded in this range.</div>
            )}
            {testFiles.length > 0 ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {testStats.productionLines > 0 ? (
                  <div
                    style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}
                    data-testid="diff-view-tests-ratio"
                  >
                    Test/code line ratio: {(testStats.ratio * 100).toFixed(0)}%
                  </div>
                ) : null}
                {onOpenFile ? (
                  <ChangeAnalysisFileTree
                    files={testFiles}
                    target={tabKey}
                    onOpenFile={(path, opts) => onOpenFile?.(path, opts)}
                    onOpenFileDiff={(path) => openDiffAt(path, 1)}
                    showFileCount={false}
                  />
                ) : null}
              </div>
            ) : null}
          </div>
        )}
      </section>

      {hasFunctionChanges && onOpenFile ? (
        <section data-testid="diff-view-function-changes">
          <h2 style={h2Style}>Function Changes</h2>
          <FunctionsCard
            boxless
            functions={functionsForList}
            churn={analysis.functionChurn}
            target={tabKey}
            onOpenFile={(path, opts) => onOpenFile(path, opts)}
            onOpenFunctionDiff={(path, line) => openDiffAt(path, line)}
          />
        </section>
      ) : null}

      {/* "The rest" of the change analysis — co-change, churn, code smells,
          duplication. Treemap + Look-here-first are hoisted above; the
          Files/Functions panel is the page's own sections. */}
      {analysis.files.length > 0 && onOpenFile ? (
        <ChangeAnalysisPanel
          analysis={analysis}
          target={tabKey}
          showHeader={false}
          showFilesPanel={false}
          showTreemap={false}
          showLookHere={false}
          onOpenPage={onOpenPage}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      ) : null}
    </div>
  );
}

/** One overlapping effort, resolved to its task title. */
interface EffortRow {
  effort: EffortAtSnapshot;
  taskTitle: string;
  /** When the effort ended (ISO), or null if still open. Used to drop
   *  long-ended efforts from the concurrent list. */
  endedAt: string | null;
}

interface EndpointDisplay {
  /** Human time label (snapshot capture time, "working tree", etc.), or
   *  null when the endpoint is identified solely by a commit. */
  timeText: string | null;
  /** Git commit this endpoint maps to, when any (shown as a linked short
   *  sha after the time). */
  commitSha: string | null;
  /** Raw capture timestamp (ISO) when this is a time-based endpoint, so
   *  the range can collapse a same-day end to time-only. Null otherwise. */
  iso: string | null;
}

/** Resolve a diff endpoint to its title-row display: a time label plus an
 *  optional git commit. */
function endpointDisplay(
  ep: DiffEndpoint | null,
  snapshotsById: Map<number, Snapshot>,
): EndpointDisplay {
  if (ep === null) return { timeText: "(initial)", commitSha: null, iso: null };
  switch (ep.kind) {
    case "working":
      return { timeText: "working tree", commitSha: null, iso: null };
    case "commit":
      return { timeText: null, commitSha: ep.sha, iso: null };
    case "snapshot": {
      const snap = snapshotsById.get(ep.snapshot_id);
      return {
        timeText: snap ? formatFullDateTime(snap.createdAt) : `snapshot ${ep.snapshot_id}`,
        commitSha: snap?.gitCommit ?? null,
        iso: snap?.createdAt ?? null,
      };
    }
  }
}

/** Plain-text endpoint label for the chrome/tab title. */
function endpointPlain(d: EndpointDisplay): string {
  const sha = d.commitSha ? d.commitSha.slice(0, 7) : null;
  if (d.timeText && sha) return `${d.timeText} (${sha})`;
  if (d.timeText) return d.timeText;
  if (sha) return sha;
  return "?";
}

/** `<start> – <end>` with each endpoint's commit (when any) rendered as a
 *  linked short sha in parentheses. */
function RangeLabel({
  start,
  end,
  onOpenCommit,
}: {
  start: EndpointDisplay;
  end: EndpointDisplay;
  onOpenCommit(sha: string): void;
}) {
  return (
    <>
      <EndpointSpan d={start} onOpenCommit={onOpenCommit} />
      <span style={{ color: "var(--text-muted)", margin: "0 6px" }}>–</span>
      <EndpointSpan d={end} onOpenCommit={onOpenCommit} />
    </>
  );
}

function EndpointSpan({
  d,
  onOpenCommit,
}: {
  d: EndpointDisplay;
  onOpenCommit(sha: string): void;
}) {
  const link = d.commitSha ? (
    <button type="button" onClick={() => onOpenCommit(d.commitSha!)} style={commitLinkStyle}>
      {d.commitSha.slice(0, 7)}
    </button>
  ) : null;
  if (d.timeText && link) return <>{d.timeText} ({link})</>;
  if (d.timeText) return <>{d.timeText}</>;
  if (link) return link;
  return <>?</>;
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
const h1Style: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-2xl)",
  fontWeight: 700,
  color: "var(--text-primary)",
  lineHeight: 1.2,
};
const h2Style: React.CSSProperties = {
  margin: "0 0 8px",
  fontSize: "var(--text-lg)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const subtitleStyle: React.CSSProperties = {
  fontSize: "var(--text-sm)",
  color: "var(--text-secondary)",
  fontFamily: "var(--mono, monospace)",
};
const effortListStyle: React.CSSProperties = {
  margin: "4px 0 0",
  paddingLeft: 18,
  display: "flex",
  flexDirection: "column",
  gap: 2,
};
const commitLinkStyle: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  font: "inherit",
  cursor: "pointer",
  textDecoration: "underline",
};
