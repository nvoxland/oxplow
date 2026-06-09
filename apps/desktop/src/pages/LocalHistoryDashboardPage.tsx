import { useCallback, useEffect, useMemo, useState } from "react";
import type { CommitRefLabel, EffortAtSnapshot, Snapshot, Stream } from "../api.js";
import {
  getSnapshotStats,
  getTaskSummaries,
  listEffortsAtSnapshots,
  listSnapshots,
  listWikiSlugsForSnapshots,
  resolveCommitRefLabels,
  subscribeGitRefsEvents,
  subscribeSnapshotEvents,
} from "../api.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { FileStatusCounts } from "../components/FileStatusCounts.js";
import { RefBadge } from "../components/RefBadge.js";
import { formatShortDateTime } from "../components/format.js";
import { logUi } from "../logger.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import type { NavSiblingEntry, NavSiblings } from "../tabs/PageNavigationContext.js";
import { gitCommitRef, indexRef, snapshotRef } from "../tabs/pageRefs.js";

const RECENT_LIMIT = 20;
/** Cap on the number of commit groups rendered in the dashboard's
 *  "By git commit" view. Older groups are dropped (Uncommitted
 *  always stays at the top regardless of cap). The dedicated
 *  "All commits" page lifts this cap. */
const BY_COMMIT_GROUP_LIMIT = 10;
/** Cap used by the dedicated full-history pages (both list and
 *  by-commit). Larger than the dashboard's RECENT_LIMIT but still
 *  bounded — pagination would come next when this stops being
 *  enough. */
const FULL_HISTORY_LIMIT = 500;

/** "dashboard" — the default landing view with both modes available
 *  via toggle, capped lists.
 *  "full-list" — dedicated page locked to the recent-snapshots
 *  layout with all snapshots up to FULL_HISTORY_LIMIT.
 *  "full-by-commit" — same idea, locked to the by-commit grouping. */
export type LocalHistoryMode = "dashboard" | "full-list" | "full-by-commit";

export interface LocalHistoryDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean; siblings?: NavSiblings }): void;
  /** Controls layout + fetch limits. Defaults to "dashboard"
   *  (capped, toggle-able). The "full-*" modes are used by the
   *  dedicated index pages reachable via the dashboard's footer
   *  links. */
  mode?: LocalHistoryMode;
}

interface SnapshotRowEffort {
  effortId: string;
  tasksId: string;
  title: string;
}

interface SnapshotRow {
  snapshot: Snapshot;
  summary: { created: number; modified: number; deleted: number; total: number } | null;
  /** Efforts that ended exactly at this snapshot (just-completed). */
  completedEfforts: SnapshotRowEffort[];
  /** Efforts that were active at this snapshot but ended later (or
   *  are still open). Surfaces as "in flight" labels on the row. */
  inFlightEfforts: SnapshotRowEffort[];
  /** Wiki page slugs whose `.md` body changed in this snapshot. */
  wikiSlugs: string[];
  /** True when this is the very first snapshot recorded for the
   *  stream — rendered as "Initial Snapshot" rather than the
   *  catch-all "External change" label. We can only assert this
   *  when the window we fetched is smaller than RECENT_LIMIT (i.e.
   *  no older snapshots scrolled off). */
  isInitial: boolean;
}

/** Pure label resolver for the snapshot row subject text. Extracted
 *  so the if/else logic is testable without a Card render.
 *
 *  Composition rule:
 *  - Completed efforts win the prefix; in-flight efforts are
 *    appended.
 *  - When neither list has anything, fall back to "Initial Snapshot"
 *    (only on the first snapshot in the stream) OR "External change"
 *    (otherwise) — but only when no other badge on the row carries
 *    meaning. `hasOtherBadges` (true for rows tagged with a git
 *    commit ref label or wiki page edit) suppresses the
 *    "External change" string and returns empty so the badges speak
 *    on their own. */
export function formatSnapshotSubject(
  completed: ReadonlyArray<{ title: string }>,
  inFlight: ReadonlyArray<{ title: string }>,
  isInitial: boolean,
  hasOtherBadges: boolean = false,
): string {
  const parts: string[] = [];
  if (completed.length > 0) {
    parts.push(`completed: ${completed.map((e) => e.title).join(", ")}`);
  }
  if (inFlight.length > 0) {
    parts.push(`in flight: ${inFlight.map((e) => e.title).join(", ")}`);
  }
  if (parts.length > 0) return parts.join(" · ");
  if (isInitial) return "Initial Snapshot";
  if (hasOtherBadges) return "";
  return "External change";
}

interface DashboardData {
  rows: SnapshotRow[];
  /** All branch+tag labels per snapshot's git commit sha. Absent shas
   *  fall back to a short-sha chip. */
  refLabels: Record<string, CommitRefLabel[]>;
}

/**
 * Local History dashboard — analogue of GitDashboardPage but driven
 * by snapshot rows (one per `request_snapshot()` call) instead of
 * git commits. Replaces the legacy per-file SnapshotsPanel.
 *
 * Layout mirrors GitDashboardPage: scrollable column of Cards. Each
 * card surfaces a different cut of the snapshot history; click into a
 * row to land on `SnapshotDetailPage` for the full file list and
 * per-file diff/restore.
 */
export function LocalHistoryDashboardPage({
  stream,
  onOpenPage,
  mode = "dashboard",
}: LocalHistoryDashboardPageProps) {
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const streamId = stream?.id ?? null;
  const fetchLimit = mode === "dashboard" ? RECENT_LIMIT : FULL_HISTORY_LIMIT;

  const refresh = useCallback(async () => {
    if (!streamId) {
      setData(null);
      setLoading(false);
      return;
    }
    try {
      setError(null);
      const snapshots = await listSnapshots(streamId, fetchLimit);
      const snapshotIds = snapshots.map((s) => s.id);
      const [summaries, effortsAt, wikiPairs] = await Promise.all([
        Promise.all(
          snapshotIds.map(async (id) => {
            try {
              return [id, await getSnapshotStats(id)] as const;
            } catch (err) {
              logUi("warn", "snapshot stats fetch failed", { error: String(err), id });
              return [id, null] as const;
            }
          }),
        ),
        listEffortsAtSnapshots(snapshotIds).catch((err): EffortAtSnapshot[] => {
          logUi("warn", "efforts-at-snapshots fetch failed", { error: String(err) });
          return [];
        }),
        listWikiSlugsForSnapshots(snapshotIds).catch((err) => {
          logUi("warn", "wiki-slugs-for-snapshots fetch failed", { error: String(err) });
          return [] as Array<{ snapshotId: number; slug: string }>;
        }),
      ]);
      const wikiBySnap = new Map<number, string[]>();
      for (const { snapshotId, slug } of wikiPairs) {
        const list = wikiBySnap.get(snapshotId) ?? [];
        if (!list.includes(slug)) list.push(slug);
        wikiBySnap.set(snapshotId, list);
      }
      const summaryById = new Map<number, { created: number; modified: number; deleted: number; total: number }>();
      for (const [id, s] of summaries) {
        if (s) summaryById.set(id, s);
      }
      // Resolve task titles for every effort the dashboard will show
      // — the efforts IPC only carries effort columns, no task title.
      const uniqueTaskIds = Array.from(new Set(effortsAt.map((e) => e.tasksId)));
      const taskSummaries = await getTaskSummaries(uniqueTaskIds).catch((err) => {
        logUi("warn", "task summaries fetch failed", { error: String(err) });
        return [] as Array<{ id: string; title: string }>;
      });
      const titleByTaskId = new Map<string, string>(
        taskSummaries.map((t) => [t.id, t.title] as [string, string]),
      );
      const completedBySnap = new Map<number, SnapshotRowEffort[]>();
      const inFlightBySnap = new Map<number, SnapshotRowEffort[]>();
      for (const e of effortsAt) {
        const target = e.completedHere ? completedBySnap : inFlightBySnap;
        const list = target.get(e.snapshotId) ?? [];
        list.push({
          effortId: e.effortId,
          tasksId: e.tasksId,
          title: titleByTaskId.get(e.tasksId) ?? `task ${e.tasksId}`,
        });
        target.set(e.snapshotId, list);
      }
      // The earliest snapshot in our window is the stream's first
      // snapshot only when we've fetched the entire history (no
      // older rows scrolled past `fetchLimit`). Without that guard
      // we'd falsely label the oldest visible row as "Initial".
      const sawFullHistory = snapshots.length < fetchLimit;
      const earliestId = sawFullHistory && snapshots.length > 0
        ? snapshots.reduce((min, s) => (s.id < min ? s.id : min), snapshots[0].id)
        : null;
      const rows: SnapshotRow[] = snapshots.map((snapshot) => ({
        snapshot,
        summary: summaryById.get(snapshot.id) ?? null,
        completedEfforts: completedBySnap.get(snapshot.id) ?? [],
        inFlightEfforts: inFlightBySnap.get(snapshot.id) ?? [],
        wikiSlugs: wikiBySnap.get(snapshot.id) ?? [],
        isInitial: earliestId !== null && snapshot.id === earliestId,
      }));
      const commitShas = Array.from(
        new Set(snapshots.map((s) => s.gitCommit).filter((sha): sha is string => Boolean(sha))),
      );
      const refLabels = await resolveCommitRefLabels(commitShas).catch(() => ({}));
      setData({ rows, refLabels });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [streamId, fetchLimit]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  // Snapshot events fire on every `request_snapshot()` flush; the
  // batched event the writer emits is the one the dashboard cares
  // about. `subscribeSnapshotEvents` already coalesces both per-file
  // and batched variants into a single callback shape, so one
  // refresh covers either case.
  useEffect(() => {
    if (!streamId) return;
    const unsub = subscribeSnapshotEvents(streamId, () => {
      void refresh();
    });
    return () => unsub();
  }, [streamId, refresh]);

  // Git refs events: a branch can move (or get created/deleted/
  // pulled) without any snapshot row changing. resolveCommitRefLabels
  // is a live git2 query, so we re-run refresh on refs events too —
  // otherwise an existing snapshot's chip would render as the
  // short-sha fallback even after the branch tip catches up to it.
  useEffect(() => {
    if (!streamId) return;
    const unsub = subscribeGitRefsEvents(streamId, () => {
      void refresh();
    });
    return () => unsub();
  }, [streamId, refresh]);

  const byBranch = useMemo(() => groupByBranch(data?.rows ?? []), [data?.rows]);

  const pageTitle =
    mode === "full-list"
      ? "All snapshots"
      : mode === "full-by-commit"
      ? "All commits"
      : "Local History";

  if (!streamId) {
    return (
      <Page testId="page-local-history" title={pageTitle}>
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-local-history" title={pageTitle}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}
        {data ? (
          <DashboardBody
            mode={mode}
            data={data}
            byBranch={byBranch}
            onOpenPage={onOpenPage}
          />
        ) : null}
      </div>
    </Page>
  );
}

type GroupingMode = "recent" | "by-commit";

function DashboardBody({
  mode: pageMode,
  data,
  byBranch,
  onOpenPage,
}: {
  mode: LocalHistoryMode;
  data: DashboardData;
  byBranch: Array<{ commit: string; rows: SnapshotRow[] }>;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean; siblings?: NavSiblings }): void;
}) {
  // Full-history pages lock the layout to a single view (no toggle,
  // no caps, no "view full" link — they ARE the full view). The
  // dashboard mode keeps the toggle + caps + footer links.
  const isFullList = pageMode === "full-list";
  const isFullByCommit = pageMode === "full-by-commit";
  const isDashboard = !isFullList && !isFullByCommit;

  const byCommitAvailable = byBranch.length > 0;
  const [toggleMode, setToggleMode] = useState<GroupingMode>("recent");
  const effectiveMode: GroupingMode = isFullList
    ? "recent"
    : isFullByCommit
    ? "by-commit"
    : toggleMode === "by-commit" && !byCommitAvailable
    ? "recent"
    : toggleMode;

  // Cap the by-commit list to the last N commit groups. Uncommitted
  // (if present) sits at the top and is not counted against the cap
  // since it's not really a commit. The full page lifts the cap.
  const cappedByBranch = useMemo(() => {
    if (isFullByCommit) return byBranch;
    const uncommitted = byBranch.filter((g) => g.commit === UNCOMMITTED_GROUP_KEY);
    const commits = byBranch.filter((g) => g.commit !== UNCOMMITTED_GROUP_KEY);
    return [...uncommitted, ...commits.slice(0, BY_COMMIT_GROUP_LIMIT)];
  }, [byBranch, isFullByCommit]);
  const commitOverflow = useMemo(
    () =>
      isFullByCommit
        ? 0
        : Math.max(
            0,
            byBranch.filter((g) => g.commit !== UNCOMMITTED_GROUP_KEY).length -
              BY_COMMIT_GROUP_LIMIT,
          ),
    [byBranch, isFullByCommit],
  );
  // The recent-list cap mirrors RECENT_LIMIT for the dashboard. The
  // full page renders all fetched rows.
  const recentRows = data.rows;
  const recentOverflow = 0; // RECENT_LIMIT is the fetch ceiling so there's nothing past it to count.

  return (
    <>
      {isDashboard ? (
        <GroupingToggle
          mode={effectiveMode}
          onChange={setToggleMode}
          byCommitAvailable={byCommitAvailable}
        />
      ) : null}
      {effectiveMode === "recent" ? (
        <RecentSnapshotsCard
          rows={recentRows}
          overflowCount={recentOverflow}
          showViewAllLink={isDashboard}
          onSelect={(id, siblings) => onOpenPage(snapshotRef(id), { siblings })}
          onViewAll={() => onOpenPage(indexRef("local-history-full"))}
          refLabels={data.refLabels}
        />
      ) : (
        <ByBranchCard
          groups={cappedByBranch}
          overflowCount={commitOverflow}
          showViewAllLink={isDashboard}
          onSelect={(id, siblings) => onOpenPage(snapshotRef(id), { siblings })}
          onViewAll={() => onOpenPage(indexRef("local-history-by-commit-full"))}
          refLabels={data.refLabels}
          onOpenCommit={(sha) => onOpenPage(gitCommitRef(sha))}
        />
      )}
    </>
  );
}

function GroupingToggle({
  mode,
  onChange,
  byCommitAvailable,
}: {
  mode: GroupingMode;
  onChange(next: GroupingMode): void;
  byCommitAvailable: boolean;
}) {
  // Sticky to the top of the scroll area so the toggle stays
  // reachable as the user scrolls through long lists. Without this
  // the toggle scrolls off and the only way back to the other view
  // is browser-back (or reloading the tab).
  return (
    <div
      style={{
        position: "sticky",
        top: 0,
        zIndex: 1,
        background: "var(--surface-card)",
        paddingBottom: 4,
        marginBottom: -4,
      }}
    >
      <div
        role="tablist"
        aria-label="Local history grouping"
        style={{
          display: "inline-flex",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          overflow: "hidden",
        }}
      >
        <ToggleButton
          active={mode === "recent"}
          onClick={() => onChange("recent")}
          label="Recent snapshots"
        />
        <ToggleButton
          active={mode === "by-commit"}
          onClick={() => onChange("by-commit")}
          label="By git commit"
          disabled={!byCommitAvailable}
          disabledTitle="No snapshots with two or more rows sharing a git commit yet."
        />
      </div>
    </div>
  );
}

function ToggleButton({
  active,
  onClick,
  label,
  disabled,
  disabledTitle,
}: {
  active: boolean;
  onClick(): void;
  label: string;
  disabled?: boolean;
  disabledTitle?: string;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      disabled={disabled}
      title={disabled ? disabledTitle : undefined}
      style={{
        padding: "4px 12px",
        fontSize: "var(--text-xs)",
        background: active ? "var(--surface-rail, var(--surface-app))" : "transparent",
        color: disabled
          ? "var(--text-muted)"
          : active
          ? "var(--text-primary)"
          : "var(--text-secondary)",
        fontWeight: active ? 600 : 400,
        border: "none",
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      {label}
    </button>
  );
}

function snapshotSiblingEntries(rows: SnapshotRow[]): NavSiblingEntry[] {
  return rows.map((row) => {
    const hasOtherBadges =
      !!row.snapshot.gitCommit || row.wikiSlugs.length > 0;
    const label = formatSnapshotSubject(
      row.completedEfforts,
      row.inFlightEfforts,
      row.isInitial,
      hasOtherBadges,
    );
    // When the subject is suppressed (badges-only row), fall back to
    // a short-sha or wiki-slug label so the prev/next tooltip still
    // says something meaningful.
    const fallback =
      row.snapshot.gitCommit?.slice(0, 7) ??
      (row.wikiSlugs.length > 0 ? `wiki:${row.wikiSlugs[0]}` : "snapshot");
    return {
      ref: snapshotRef(row.snapshot.id),
      label: label || fallback,
    };
  });
}

function RecentSnapshotsCard({
  rows,
  overflowCount,
  showViewAllLink,
  onSelect,
  onViewAll,
  refLabels,
}: {
  rows: SnapshotRow[];
  /** Snapshots that exist past the visible window. Currently always
   *  0 in practice — RECENT_LIMIT is the fetch ceiling. Accepted
   *  here for symmetry with ByBranchCard's overflow indicator. */
  overflowCount: number;
  /** When true, render the "View all snapshots →" footer link. */
  showViewAllLink: boolean;
  onSelect(id: number, siblings: NavSiblings): void;
  onViewAll(): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  const entries = useMemo(() => snapshotSiblingEntries(rows), [rows]);
  return (
    <Card testId="local-history-recent" title="Recent Snapshots">
      {rows.length === 0 ? (
        <div style={muted}>No snapshots yet.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {rows.map((row, idx) => (
            <SnapshotRowItem
              key={row.snapshot.id}
              row={row}
              onSelect={(id) => onSelect(id, { entries, index: idx, title: "Recent snapshots" })}
              labels={row.snapshot.gitCommit ? refLabels[row.snapshot.gitCommit] ?? [] : []}
            />
          ))}
        </div>
      )}
      {showViewAllLink ? (
        <div style={{ marginTop: 8 }}>
          <button
            type="button"
            onClick={onViewAll}
            style={cardLinkButton}
            title="Open the full snapshot history in its own tab"
          >
            View all snapshots →
            {overflowCount > 0 ? ` (${overflowCount} more)` : ""}
          </button>
        </div>
      ) : null}
    </Card>
  );
}

function SnapshotRowItem({
  row,
  onSelect,
  labels,
  hideVersionChip = false,
}: {
  row: SnapshotRow;
  onSelect(id: number): void;
  labels: CommitRefLabel[];
  /** When true, suppress the per-row branch/tag/sha chip. Used by
   *  the "By git commit" view where the group header already shows
   *  it for every row in the group. */
  hideVersionChip?: boolean;
}) {
  const { snapshot, summary, completedEfforts, inFlightEfforts, wikiSlugs, isInitial } = row;
  // A git_commit on the snapshot always renders at least a short-sha
  // chip (or branch/tag chips when ref labels resolve), and wiki
  // badges similarly carry meaning on their own — both suppress the
  // "External change" fallback because the chips speak for themselves.
  // The suppression still applies even when the chip itself is
  // hidden via hideVersionChip — the group header above carries the
  // version context.
  const hasOtherBadges = !!snapshot.gitCommit || wikiSlugs.length > 0;
  const subjectish = formatSnapshotSubject(
    completedEfforts,
    inFlightEfforts,
    isInitial,
    hasOtherBadges,
  );
  return (
    <button
      type="button"
      data-testid="local-history-snapshot-row"
      onClick={() => onSelect(snapshot.id)}
      style={rowButtonStyle}
      title="Open snapshot detail"
    >
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {subjectish}
      </span>
      {hideVersionChip ? null : labels.length > 0
        ? labels.map((l) => (
            <RefBadge key={`${l.kind}-${l.name}`} label={l.name} tone={l.kind} />
          ))
        : snapshot.gitCommit
        ? <RefBadge label={snapshot.gitCommit.slice(0, 7)} tone="sha" />
        : null}
      {wikiSlugs.map((slug) => (
        <RefBadge key={`wiki-${slug}`} label={slug} tone="wiki" />
      ))}
      {summary ? (
        <FileStatusCounts
          filesAdded={summary.created}
          filesModified={summary.modified}
          filesDeleted={summary.deleted}
          title={`${summary.total} file${summary.total === 1 ? "" : "s"} captured: ${summary.created} created · ${summary.modified} modified · ${summary.deleted} deleted`}
        />
      ) : null}
      <span style={{ ...subtle, width: 130, flexShrink: 0, textAlign: "right" }} title={snapshot.createdAt}>
        {formatShortDateTime(snapshot.createdAt)}
      </span>
    </button>
  );
}

function ByBranchCard({
  groups,
  overflowCount,
  showViewAllLink,
  onSelect,
  onViewAll,
  onOpenCommit,
  refLabels,
}: {
  groups: Array<{ commit: string; rows: SnapshotRow[] }>;
  /** Commit groups beyond the visible cap. When non-zero and
   *  showViewAllLink is true, the "View all commits →" link
   *  surfaces the count. */
  overflowCount: number;
  showViewAllLink: boolean;
  onSelect(id: number, siblings: NavSiblings): void;
  onViewAll(): void;
  onOpenCommit(sha: string): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  return (
    <Card testId="local-history-by-branch" title="By Git Commit">
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {groups.map((group) => (
          <ByBranchGroup
            key={group.commit}
            group={group}
            onSelect={onSelect}
            onOpenCommit={onOpenCommit}
            refLabels={refLabels}
          />
        ))}
      </div>
      {showViewAllLink ? (
        <div style={{ marginTop: 8 }}>
          <button
            type="button"
            onClick={onViewAll}
            style={cardLinkButton}
            title="Open the full by-commit history in its own tab"
          >
            View all commits →
            {overflowCount > 0 ? ` (${overflowCount} more)` : ""}
          </button>
        </div>
      ) : null}
    </Card>
  );
}

function ByBranchGroup({
  group,
  onSelect,
  onOpenCommit,
  refLabels,
}: {
  group: { commit: string; rows: SnapshotRow[] };
  onSelect(id: number, siblings: NavSiblings): void;
  onOpenCommit(sha: string): void;
  refLabels: Record<string, CommitRefLabel[]>;
}) {
  const entries = useMemo(() => snapshotSiblingEntries(group.rows), [group.rows]);
  const isUncommitted = group.commit === UNCOMMITTED_GROUP_KEY;
  const groupLabels = isUncommitted ? [] : refLabels[group.commit] ?? [];
  const headerName = isUncommitted
    ? "Uncommitted"
    : groupLabels.length > 0
    ? groupLabels.map((l) => l.name).join(", ")
    : group.commit.slice(0, 7);
  const title = isUncommitted
    ? "Uncommitted snapshots"
    : `Snapshots at ${headerName}`;
  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
        {isUncommitted ? (
          <span
            style={{
              fontWeight: 600,
              fontSize: 12,
              color: "var(--text-secondary)",
            }}
            title="Snapshots taken while the worktree had uncommitted changes — no git commit recorded yet."
          >
            Uncommitted
          </span>
        ) : groupLabels.length > 0 ? (
          groupLabels.map((l) => (
            <RefBadge key={`${l.kind}-${l.name}`} label={l.name} tone={l.kind} />
          ))
        ) : (
          <button
            type="button"
            onClick={() => onOpenCommit(group.commit)}
            style={{ ...cardLinkButton, fontFamily: "monospace" }}
          >
            {group.commit.slice(0, 7)}
          </button>
        )}
        {!isUncommitted && groupLabels.length > 0 ? (
          <button
            type="button"
            onClick={() => onOpenCommit(group.commit)}
            style={{ ...cardLinkButton, fontFamily: "monospace", fontSize: 11 }}
            title={`commit ${group.commit}`}
          >
            {group.commit.slice(0, 7)}
          </button>
        ) : null}
        <span style={subtle}>· {group.rows.length} snapshots</span>
      </div>
      <div style={{ display: "flex", flexDirection: "column" }}>
        {group.rows.map((row, idx) => (
          <SnapshotRowItem
            key={row.snapshot.id}
            row={row}
            onSelect={(id) => onSelect(id, { entries, index: idx, title })}
            labels={row.snapshot.gitCommit ? refLabels[row.snapshot.gitCommit] ?? [] : []}
            hideVersionChip
          />
        ))}
      </div>
    </div>
  );
}


/** Sentinel `commit` value used by groupByBranch for snapshots
 *  with no recorded git_commit (worktree was dirty at capture). The
 *  ByBranchGroup renderer detects this and shows an "Uncommitted"
 *  header instead of a sha. */
export const UNCOMMITTED_GROUP_KEY = "__uncommitted__";

function groupByBranch(
  rows: SnapshotRow[],
): Array<{ commit: string; rows: SnapshotRow[] }> {
  // `rows` arrives newest-first from listSnapshots. The first
  // occurrence of each commit (or the uncommitted sentinel) is its
  // most recent snapshot, so Map insertion order yields
  // "most recent first." Snapshots without a git_commit cluster
  // under the sentinel; when present, that group is hoisted to the
  // top so "what's happened since the last commit" reads first.
  const byCommit = new Map<string, SnapshotRow[]>();
  for (const row of rows) {
    const key = row.snapshot.gitCommit ?? UNCOMMITTED_GROUP_KEY;
    const existing = byCommit.get(key) ?? [];
    existing.push(row);
    byCommit.set(key, existing);
  }
  const groups = Array.from(byCommit.entries()).map(([commit, rs]) => ({
    commit,
    rows: rs,
  }));
  // Hoist Uncommitted to the top regardless of insertion order.
  groups.sort((a, b) => {
    if (a.commit === UNCOMMITTED_GROUP_KEY) return -1;
    if (b.commit === UNCOMMITTED_GROUP_KEY) return 1;
    return 0;
  });
  return groups;
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const rowButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "4px 6px",
  background: "transparent",
  border: "none",
  borderBottom: "1px solid var(--border-subtle)",
  cursor: "pointer",
  textAlign: "left",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
};
