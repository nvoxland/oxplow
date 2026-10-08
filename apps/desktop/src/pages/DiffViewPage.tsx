import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { DiffEntry, EffortAtSnapshot, Snapshot, Stream } from "../api.js";
import type { ExtensionChange } from "../tauri-bridge/generated/bindings.js";
import { ImpactReportView } from "../components/ImpactReportView.js";
import { readWorkItem, readWorkItemsByRef } from "../workItems.js";
import {
  extensionImpactBetween,
  querySql,
  getAgentTurn,
  getEffort,
  listEffortFiles,
  listEffortsOverlappingRange,
  listSnapshots,
} from "../api.js";
import {
  changedExtensions,
  pickerBranch,
  previousSnapshotId,
  rangeDateLabel,
  rangeEndpointOptions,
  inProgressNotice,
  resolveEffortEndpoints,
  resolveSnapshotEndpoints,
  resolveTurnEndpoints,
  snapshotRange,
  type DiffSubject,
  snapshotsOnBranch,
} from "../diffViewModel.js";
import { WORKING, snapshotIdOf, snapshotRevision, vcsRevOf, type Revision } from "../revision.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { Page, pageH1Style } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import {
  effortDiffRef,
  endpointDiffRef,
  snapshotRef,
  workItemTabRef,
  turnRef,
  type DiffViewPayload,
} from "../tabs/pageRefs.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { ChangedFilesTree } from "../components/ChangedFiles/ChangedFilesTree.js";
import { useChangedFiles } from "../components/ChangedFiles/useChangedFiles.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { LensSlots } from "../lens/LensSlots.js";
import { useChange } from "../lens/useChange.js";
import { effortRowId } from "../lens/lensModel.js";
import { EffortVerdict } from "./EffortVerdictStrip.js";
import { EndpointPicker, type EndpointSnapshotOption } from "../components/Diff/EndpointPicker.js";
import { formatFullDateTime, formatTimeOnly } from "../components/format.js";
import { workItemLabel } from "../workItemRef.js";
import { EffortHeader } from "./EffortHeader.js";

/**
 * What a diff view renders. Reached four ways, all via `DiffViewPage`:
 *
 * - `snapshot` — the `snapshot:<N>` page: a parent→N diff of a single
 *   capture (its recorded parent; full function/duplication analysis).
 * - `effort` — the `effort:<effN>` page: the effort's own start/end
 *   snapshot bracket, with the task title + an "in progress" notice when
 *   the effort is still open.
 * - `turn` — the `turn:<trnN>` page: what an agent turn changed, its
 *   start snapshot → its end snapshot (start → working tree while it runs).
 * - `endpoints` — `endpointDiffRef(start, end)`: an explicit pair of
 *   snapshot/commit/working endpoints diffed via the unified substrate.
 */
/** What the page diffs — the payload of a snapshot / effort / turn page
 *  or a `diff-view` endpoints route (`tabs/pageRefs.ts`). */
export type DiffViewSpec = DiffViewPayload;

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
  start: Revision | null;
  end: Revision;
  inProgress: boolean;
  /** What the diff is of — picks the in-progress notice's wording. */
  subject: DiffSubject;
  /** The work item of the effort this diff was opened *for* (effort
   *  mode); null for snapshot/endpoint diffs and an unlinked effort. */
  workItem: string | null;
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
  // Why nothing resolved: an error, or a turn with nothing to diff.
  const [unresolved, setUnresolved] = useState<string | null>(null);
  const key = specKey(spec);

  // Backlinks/outbound keyed on the ref that opened this page. A
  // snapshot is a linkable entity; effort/endpoint diffs rarely are,
  // but a stable identity keeps the page chrome uniform.
  const graphRef = useMemo<TabRef>(() => {
    if (spec.mode === "snapshot") return snapshotRef(spec.snapshotId);
    if (spec.mode === "effort") return effortDiffRef(spec.effortId);
    if (spec.mode === "turn") return turnRef(spec.turnId);
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
    setUnresolved(null);
    if (spec.mode === "endpoints") {
      setResolved({
        start: spec.start,
        end: spec.end,
        inProgress: spec.end === WORKING,
        subject: "endpoints",
        workItem: null,
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
            setUnresolved("Effort not found.");
            return;
          }
          setResolved({
            ...resolveEffortEndpoints(effort),
            subject: "effort",
            workItem: effort.workItem,
            effortId: effort.effortId,
          });
        })
        .catch((err) => {
          if (cancelled) return;
          logUi("warn", "effort resolve failed", { error: String(err) });
          setUnresolved(err instanceof Error ? err.message : String(err));
        });
      return () => {
        cancelled = true;
      };
    }
    if (spec.mode === "turn") {
      // A turn diffs its start snapshot → its end snapshot; one still
      // running diffs its start against the working tree.
      void getAgentTurn(spec.turnId)
        .then((turn) => {
          if (cancelled) return;
          if (!turn) {
            setUnresolved("Turn not found.");
            return;
          }
          const endpoints = resolveTurnEndpoints(turn);
          if ("unavailable" in endpoints) {
            setUnresolved(endpoints.unavailable);
            return;
          }
          setResolved({
            ...endpoints,
            subject: "turn",
            workItem: null,
            effortId: null,
          });
        })
        .catch((err) => {
          if (cancelled) return;
          logUi("warn", "turn resolve failed", { error: String(err) });
          setUnresolved(err instanceof Error ? err.message : String(err));
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
        const prev = previousSnapshotId(snapshotId, rows);
        setResolved({
          ...resolveSnapshotEndpoints(snapshotId, prev),
          subject: "endpoints",
          workItem: null,
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

  // Loading / error keep the simple full-layout chrome; the resolved view
  // owns its own Page (it needs the range data for the details rail).
  if (unresolved || !resolved) {
    return (
      <Page testId="page-diff-view" title="Changes" kind="diff-view" backlinks={backlinks} outbound={outbound}>
        <div style={{ ...muted, padding: "12px 16px" }}>{unresolved ?? "Loading…"}</div>
      </Page>
    );
  }
  return (
    <ResolvedEndpointDiff
      stream={stream}
      resolved={resolved}
      backlinks={backlinks}
      outbound={outbound}
      onOpenPage={onOpenPage}
      onOpenFile={onOpenFile}
      onOpenDiff={onOpenDiff}
      onOpenDiffInTab={onOpenDiffInTab}
    />
  );
}

function specKey(spec: DiffViewSpec): string {
  switch (spec.mode) {
    case "snapshot":
      return `snapshot:${spec.snapshotId}`;
    case "effort":
      return `effort:${spec.effortId}`;
    case "turn":
      return `turn:${spec.turnId}`;
    case "endpoints":
      return `endpoints:${JSON.stringify(spec.start)}:${JSON.stringify(spec.end)}`;
  }
}

function ResolvedEndpointDiff({
  stream,
  resolved,
  backlinks,
  outbound,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: {
  stream: Stream | null;
  resolved: ResolvedDiff;
  backlinks: { count: number; body: ReactNode };
  outbound?: { count: number; body: ReactNode };
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
}) {
  const { start, end, inProgress, workItem, effortId } = resolved;
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

  // The changed files. An in-progress effort diffs its start snapshot
  // against the live working tree (the `working` endpoint); a small
  // header note flags that the end side is moving.
  const changed = useChangedFiles(stream ? { kind: "endpoints", streamId: stream.id, start, end } : null);

  // An extension's files changed: what the change does, each
  // version reviewed as its revision holds it.
  const extensionNames = changedExtensions(changed.files.map((f) => f.path));
  const extensionKey = extensionNames.join(",");
  const [extensionReview, setExtensionReview] = useState<
    { state: "reviewing" } | { state: "reviewed"; changes: ExtensionChange[] } | { state: "failed"; message: string }
  >({ state: "reviewing" });
  useEffect(() => {
    setExtensionReview({ state: "reviewing" });
    if (!stream || extensionKey === "") return;
    let cancelled = false;
    void extensionImpactBetween(stream.id, start, end)
      .then((changes) => {
        if (!cancelled) setExtensionReview({ state: "reviewed", changes });
      })
      .catch((err: unknown) => {
        logUi("warn", "extension impact failed", { error: String(err) });
        if (!cancelled) {
          setExtensionReview({ state: "failed", message: err instanceof Error ? err.message : String(err) });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, start, end, extensionKey]);

  // The work item's title for the header (effort mode).
  const [taskTitle, setTaskTitle] = useState<string | null>(null);
  useEffect(() => {
    if (!workItem) {
      setTaskTitle(null);
      return;
    }
    let cancelled = false;
    void readWorkItemsByRef([workItem])
      .then(({ items }) => {
        if (cancelled) return;
        setTaskTitle(items.find((r) => r.ref === workItem)?.title ?? null);
      })
      .catch(() => setTaskTitle(null));
    return () => {
      cancelled = true;
    };
  }, [workItem]);

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
        const itemRefs = overlapping.flatMap((o) => (o.workItem ? [o.workItem] : []));
        const titles = await readWorkItemsByRef(Array.from(new Set(itemRefs)))
          .then((r) => r.items)
          .catch(() => [] as Array<{ ref: string; title: string }>);
        const titleByItem = new Map(titles.map((t) => [t.ref, t.title] as const));
        // An unlinked effort is named by its own title (`v_effort.title`).
        const unlinked = overlapping
          .map((o) => (o.workItem ? null : effortRowId(o.effortId)))
          .filter((id): id is number => id !== null);
        const titleByEffort = new Map<number, string>();
        if (unlinked.length > 0) {
          const result = await querySql(
            `SELECT id, title FROM v_effort WHERE id IN (${unlinked.map((_, i) => `?${i + 1}`).join(", ")})`,
            unlinked,
          ).catch(() => null);
          for (const row of result?.rows ?? []) {
            if (row[1] !== null && row[1] !== undefined) titleByEffort.set(Number(row[0]), String(row[1]));
          }
        }
        if (cancelled) return;
        setEffortRows(
          overlapping.map((o) => ({
            effort: {
              snapshotId: range.rangeEnd,
              effortId: o.effortId,
              workItem: o.workItem,
              threadId: o.threadId,
              startSnapshotId: o.startSnapshotId,
              endSnapshotId: o.endSnapshotId,
              completedHere: o.endSnapshotId === range.rangeEnd,
            },
            taskTitle: o.workItem
              ? titleByItem.get(o.workItem) ?? workItemLabel(o.workItem)
              : titleByEffort.get(effortRowId(o.effortId) ?? -1) ?? "Unlinked work",
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
  const startSnapId = start === null ? null : snapshotIdOf(start);
  const endSnapId = snapshotIdOf(end);
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
  // The effort's stored change analysis, for effort-review lenses that
  // read v_change* (none when the effort has no start snapshot).
  const { change: effortChange } = useChange(primaryEffortId ? { kind: "effort", effortId: primaryEffortId } : null);
  // In effort mode the header reads the effort's own title (`v_effort`).
  const [ownTitle, setOwnTitle] = useState<string | null>(null);
  const effortTitle = effortPassed ? ownTitle ?? taskTitle : linedUpEffort?.taskTitle ?? null;
  const primaryItem = effortPassed ? workItem : linedUpEffort?.effort.workItem ?? null;

  // The effort's task description, rendered at the top when the diff is for
  // an effort, so the reader has its intent in context.
  const [effortDescription, setEffortDescription] = useState<string | null>(null);
  useEffect(() => {
    if (!primaryItem) {
      setEffortDescription(null);
      return;
    }
    let cancelled = false;
    void readWorkItem(primaryItem)
      .then(({ item }) => {
        if (!cancelled) setEffortDescription(item?.body ?? null);
      })
      .catch(() => {
        if (!cancelled) setEffortDescription(null);
      });
    return () => {
      cancelled = true;
    };
  }, [primaryItem]);

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
  const endDisp = useMemo(
    () => endpointDisplay(end, snapshotsById),
    [JSON.stringify(end), snapshotsById],
  );

  // Candidate snapshots for the Start/End pickers: the 20 newest captures in
  // this stream *on the same branch as the diffed endpoints* (so a branch
  // switch within the stream's worktree never mixes in other branches'
  // snapshots), constrained per side so the range stays valid — the Start
  // list stops before the End, the End list starts after the Start — while
  // always keeping the current selection visible.
  const { startOptions, endOptions } = useMemo(() => {
    const branch = pickerBranch(
      endSnapId != null ? snapshotsById.get(endSnapId) : undefined,
      startSnapId != null ? snapshotsById.get(startSnapId) : undefined,
      stream?.branch ?? null,
    );
    const onBranch = snapshotsOnBranch([...snapshotsById.values()], branch);
    const toOption = (s: Snapshot): EndpointSnapshotOption => ({
      snapshotId: s.id,
      createdAt: s.createdAt,
      commit: vcsRevOf(s.revision),
    });
    return {
      startOptions: rangeEndpointOptions(onBranch, "start", endSnapId, startSnapId, 20).map(
        toOption,
      ),
      endOptions: rangeEndpointOptions(onBranch, "end", startSnapId, endSnapId, 20).map(toOption),
    };
  }, [snapshotsById, startSnapId, endSnapId, stream?.branch]);

  // Page title. For an effort: "Changes: <effort title>". Otherwise a
  // comparison label — "Commit comparison" when BOTH sides are git versions
  // (commits / commit-pinned snapshots), else "Snapshot comparison". The
  // date/commit range itself lives in the details rail.
  const bothGit = startDisp.commitSha != null && endDisp.commitSha != null;
  const plainTitle = effortTitle
    ? `Changes: ${effortTitle}`
    : bothGit
      ? "Commit comparison"
      : "Snapshot comparison";
  usePageTitle(plainTitle);

  // Files for the Files Changed tree. All changed files by default; only
  // the effort's claimed files when a diff was opened *for* an effort.
  const filesForList = useMemo<DiffEntry[]>(() => {
    if (!effortPassed) return changed.files;
    if (!claimedPaths) return [];
    return changed.files.filter((f) => claimedPaths.has(f.path));
  }, [effortPassed, claimedPaths, changed.files]);
  const filesLoading = changed.loading || (effortPassed && claimedPaths === null);

  // Open a file's diff in the current tab (revealing `line`).
  const openDiffAt = (path: string, line = 1) => {
    if (!changed.base || !changed.head) {
      onOpenFile?.(path);
      return;
    }
    const spec: DiffSpec = {
      path,
      leftVersion: changed.base,
      rightVersion: changed.head,
      baseLabel: endpointPlain(startDisp),
      revealLine: line,
    };
    if (onOpenDiffInTab) onOpenDiffInTab(spec);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile?.(path);
  };

  // Rescope the diff in place by re-pointing one endpoint at a chosen
  // snapshot — navigates the tab to the new endpoint pair (Back returns).
  const pickStart = (snapshotId: number) =>
    onOpenPage(endpointDiffRef(snapshotRevision(snapshotId), end));
  const pickEnd = (snapshotId: number) => onOpenPage(endpointDiffRef(start, snapshotRevision(snapshotId)));

  // The date/commit range lives in the details rail: a date (or date range
  // when the endpoints span days) header above two selectable fields — a
  // "Start"/"End" caption beside a snapshot dropdown (time-only when closed,
  // full date+time + commit in the menu) that rescopes the diff when picked.
  const dateLabel = rangeDateLabel(startDisp.iso, endDisp.iso);
  const rail = (
    <div data-testid="diff-view-range" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {/* When this diff lines up with an effort, name its work item and link
          to it (effort passed, or a snapshot range that matches an
          overlapping effort's bracket — `primaryItem`/`effortTitle`). */}
      {primaryItem && effortTitle ? (
        <div style={railRowStyle}>
          <span style={railLabelStyle}>Work item</span>
          <button
            type="button"
            data-testid="diff-view-task-link"
            onClick={() => onOpenPage(workItemTabRef(primaryItem))}
            title={effortTitle}
            style={{
              ...linkButton,
              fontFamily: "inherit",
              fontSize: "var(--text-sm)",
              textAlign: "left",
              minWidth: 0,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {effortTitle}
          </button>
        </div>
      ) : null}
      {dateLabel ? (
        <div data-testid="diff-view-range-date" style={railDateStyle}>
          {dateLabel}
        </div>
      ) : null}
      <div style={railRowStyle}>
        <span style={railLabelStyle}>Start</span>
        <EndpointPicker
          testId="diff-range-start"
          ariaLabel="Start of range"
          triggerText={endpointShort(startDisp)}
          currentSnapshotId={startSnapId}
          options={startOptions}
          onPick={pickStart}
        />
      </div>
      <div style={railRowStyle}>
        <span style={railLabelStyle}>End</span>
        <EndpointPicker
          testId="diff-range-end"
          ariaLabel="End of range"
          triggerText={endpointShort(endDisp)}
          currentSnapshotId={endSnapId}
          options={endOptions}
          onPick={pickEnd}
        />
      </div>
    </div>
  );

  return (
    <Page
      testId="page-diff-view"
      kind="diff-view"
      titleInBody
      backlinks={backlinks}
      outbound={outbound}
      layout="details"
      rightRail={rail}
    >
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {effortPassed && effortId ? (
        <EffortHeader effortId={effortId} onOpenPage={(ref) => onOpenPage(ref)} onTitle={setOwnTitle} />
      ) : (
        <h1 style={pageH1Style} data-testid="diff-view-title">{plainTitle}</h1>
      )}

      {effortDescription && effortDescription.trim() ? (
        <div data-testid="diff-view-effort-description" style={{ fontSize: "var(--text-sm)" }}>
          {/* Override `.oxplow-md`'s `margin: 0 auto` so the description
              left-aligns with the page's other sections instead of centering. */}
          <MarkdownView
            body={effortDescription}
            onOpenFile={onOpenFile ? (path) => onOpenFile(path) : undefined}
            renderMermaid
            style={{ marginLeft: 0, marginRight: 0 }}
          />
        </div>
      ) : null}

      {primaryEffortId && effortRowId(primaryEffortId) !== null ? (
        <EffortVerdict effortRow={effortRowId(primaryEffortId)!} />
      ) : null}

      {primaryEffortId ? (
        <LensSlots
          slot="effort.review.details"
          params={
            effortRowId(primaryEffortId) === null
              ? null
              : { effort_id: effortRowId(primaryEffortId), ...(effortChange ? { change_id: effortChange.id } : {}) }
          }
          streamId={stream?.id ?? null}
          onOpenPage={(ref) => onOpenPage(ref)}
          h2Style={h2Style}
        />
      ) : null}

      {inProgress ? (
        <div
          style={{ ...card, color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}
          data-testid="diff-view-in-progress"
        >
          {inProgressNotice(resolved.subject)}
        </div>
      ) : null}

      {changed.error ? (
        <div style={{ ...card, color: "var(--severity-critical, #f87171)", fontSize: "var(--text-sm)" }}>
          {changed.error}
        </div>
      ) : null}

      {concurrentEfforts.length > 0 ? (
        <section data-testid="diff-view-concurrent-efforts">
          <h2 style={h2Style}>Concurrent Efforts</h2>
          <ul style={effortListStyle}>
            {concurrentEfforts.map((r) => (
              <li key={r.effort.effortId}>
                {r.effort.workItem ? (
                  <button
                    type="button"
                    onClick={() => onOpenPage(workItemTabRef(r.effort.workItem as string))}
                    style={{ ...linkButton, fontFamily: "inherit", fontSize: "var(--text-sm)" }}
                  >
                    {r.taskTitle}
                  </button>
                ) : (
                  // An unlinked effort has no item page.
                  <span style={{ fontSize: "var(--text-sm)" }}>{r.taskTitle}</span>
                )}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section data-testid="diff-view-files-changed">
        <h2 style={h2Style}>Files Changed</h2>
        {filesLoading && filesForList.length === 0 ? (
          <div style={muted}>Loading…</div>
        ) : filesForList.length === 0 ? (
          <div style={muted}>
            {effortPassed
              ? "This effort has no changed files."
              : "No file changes between these endpoints."}
          </div>
        ) : onOpenFile ? (
          <ChangedFilesTree
            files={filesForList}
            onOpenFile={(path, opts) => onOpenFile(path, opts)}
            onOpenFileDiff={(path) => openDiffAt(path, 1)}
            showFileCount={false}
          />
        ) : null}
      </section>

      {extensionNames.length > 0 ? (
        <section data-testid="diff-view-extension-changes">
          <h2 style={h2Style}>Extension Changes</h2>
          {extensionReview.state === "reviewing" ? (
            <div style={muted}>Reviewing {extensionNames.join(", ")}…</div>
          ) : extensionReview.state === "failed" ? (
            <div
              data-testid="diff-view-extension-changes-error"
              style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}
            >
              Could not review {extensionNames.join(", ")}: {extensionReview.message}
            </div>
          ) : (
            extensionReview.changes.map((c) => (
              <div key={c.name} data-testid={`extension-change-${c.name}`} style={{ marginBottom: 10 }}>
                <div style={{ fontWeight: 600 }}>
                  {c.name} <span style={muted}>{c.change}</span>
                </div>
                {c.errors.map((e, i) => (
                  <div key={i} style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}>
                    {e}
                  </div>
                ))}
                {c.impact ? <ImpactReportView report={c.impact} testId={`extension-change-${c.name}-impact`} /> : null}
              </div>
            ))
          )}
        </section>
      ) : null}

    </div>
    </Page>
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
 *  optional commit. */
function endpointDisplay(
  ep: Revision | null,
  snapshotsById: Map<number, Snapshot>,
): EndpointDisplay {
  if (ep === null) return { timeText: "(initial)", commitSha: null, iso: null };
  if (ep === WORKING) return { timeText: "working tree", commitSha: null, iso: null };
  const snapshotId = snapshotIdOf(ep);
  if (snapshotId === null) {
    return { timeText: null, commitSha: ep.slice(ep.indexOf(":") + 1), iso: null };
  }
  const snap = snapshotsById.get(snapshotId);
  return {
    timeText: snap ? formatFullDateTime(snap.createdAt) : `snapshot ${snapshotId}`,
    commitSha: vcsRevOf(snap?.revision),
    iso: snap?.createdAt ?? null,
  };
}

/** Plain-text endpoint label for the chrome/tab title. */
function endpointPlain(d: EndpointDisplay): string {
  const sha = d.commitSha ? d.commitSha.slice(0, 7) : null;
  if (d.timeText && sha) return `${d.timeText} (${sha})`;
  if (d.timeText) return d.timeText;
  if (sha) return sha;
  return "?";
}

/** Short (time-only) endpoint label for the rail's dropdown trigger: just the
 *  capture time when it's a snapshot, else the working/initial label or a
 *  short commit sha. The full date+time lives inside the dropdown. */
function endpointShort(d: EndpointDisplay): string {
  if (d.iso) return formatTimeOnly(d.iso);
  if (d.timeText) return d.timeText;
  if (d.commitSha) return d.commitSha.slice(0, 7);
  return "?";
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
const h2Style: React.CSSProperties = {
  margin: "0 0 8px",
  fontSize: "var(--text-lg)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const effortListStyle: React.CSSProperties = {
  margin: "4px 0 0",
  paddingLeft: 18,
  display: "flex",
  flexDirection: "column",
  gap: 2,
};
const railDateStyle: React.CSSProperties = {
  fontSize: "var(--text-sm)",
  fontWeight: 600,
  color: "var(--text-primary)",
  marginBottom: 2,
};
const railRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
};
const railLabelStyle: React.CSSProperties = {
  width: 38,
  flexShrink: 0,
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  textTransform: "uppercase",
  letterSpacing: 0.4,
};
