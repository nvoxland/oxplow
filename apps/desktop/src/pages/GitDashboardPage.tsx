import { EmptyState } from "../components/Prompts/EmptyState.js";
import { SpecConfirm } from "../components/SpecConfirm.js";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MergeReadiness, OpOutcome, RemoteBranchEntry, RevisionInfo, Stream, StatusCounts } from "../api.js";
import {
  countStatus,
  vcsDivergence,
  vcsHead,
  vcsRevision,
  vcsRevisionsBetween,
  vcsStatus,
  gitRebase,
  vcsFetch,
  vcsMerge,
  vcsPull,
  vcsPush,
  listAgentStatuses,
  listRecentRemoteBranches,
  listStreams,
  subscribeAgentStatus,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
} from "../api.js";
import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { gitRevision, vcsRevOf } from "../revision.js";
import { readBranches, readHistory, type History } from "../vcsHistory.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, indexRef, uncommittedChangesRef } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { awaitGitOp, gitOpErrorMessage, opErrorOf } from "../git-op.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { CommitGraphTable, indexRefsBySha, type CommitStats } from "../components/History/CommitGraphTable.js";
import { FileStatusCountsForSummary } from "../components/FileStatusCounts.js";

export interface GitDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onRevealCommit(sha: string): void;
}

interface DashboardData {
  branchHeader: {
    branch: string | null;
    headSha: string | null;
    headSubject: string | null;
    headDate: number | null;
    upstream: string | null;
    aheadUpstream: number;
    behindUpstream: number;
  };
  uncommitted: StatusCounts | null;
  recentLog: History & { currentBranch: string | null };
  streams: StreamRow[];
  remoteBranches: RemoteBranchEntry[];
  divergence: StreamDivergenceReport;
}

/** One stream's divergence from the integration branch. */
interface StreamDivergenceRow {
  streamId: string;
  title: string;
  branch: string;
  ahead: number;
  behind: number;
  overlappingFiles: string[];
  readiness: MergeReadiness;
}

/** Every stream's divergence from the integration branch `base` (the
 *  repository's default branch, from `v_branch`). */
interface StreamDivergenceReport {
  base: string;
  rows: StreamDivergenceRow[];
}

/** Compare every stream's branch with the default branch. */
async function readStreamDivergences(
  streamId: string,
  streams: Stream[],
): Promise<StreamDivergenceReport> {
  const { branches } = await readBranches();
  const base = branches.find((b) => b.remote === null && b.isDefault)?.name ?? "main";
  const rows = await Promise.all(
    streams
      .filter((s) => s.branch)
      .map(async (s): Promise<StreamDivergenceRow | null> => {
        try {
          const d = await vcsDivergence(streamId, gitRevision(base), gitRevision(s.branch));
          return {
            streamId: s.id,
            title: s.title,
            branch: s.branch,
            ahead: d.ahead,
            behind: d.behind,
            overlappingFiles: d.overlapping_files,
            readiness: d.readiness,
          };
        } catch {
          return null;
        }
      }),
  );
  return { base, rows: rows.filter((r): r is StreamDivergenceRow => r !== null) };
}

interface StreamRow {
  stream: Stream;
  branch: string | null;
  ahead: number;
  behind: number;
  uncommitted: StatusCounts | null;
}

const RECENT_LIMIT = 5;

export function GitDashboardPage({ stream, onOpenPage, onRevealCommit }: GitDashboardPageProps) {
  const nav = useOptionalPageNavigation();
  const handleSelectCommit = (sha: string) => {
    if (nav) nav.navigate(gitCommitRef(sha));
    else onRevealCommit(sha);
  };
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Set of action labels that are currently in-flight. Driven by the
  // BackgroundTaskStore: when a kickoff IPC returns its taskId we add
  // the label and subscribe; the subscription removes the label once the
  // task ends. This means buttons stay "pending" for the entire duration
  // of the underlying git op even when the IPC promise resolved long
  // ago, and any other surface watching the same store sees the same
  // state.
  const [pendingLabels, setPendingLabels] = useState<ReadonlySet<string>>(new Set());
  const isPending = useCallback((label: string) => pendingLabels.has(label), [pendingLabels]);
  const addPending = useCallback((label: string) => {
    setPendingLabels((prev) => {
      const next = new Set(prev);
      next.add(label);
      return next;
    });
  }, []);
  const removePending = useCallback((label: string) => {
    setPendingLabels((prev) => {
      if (!prev.has(label)) return prev;
      const next = new Set(prev);
      next.delete(label);
      return next;
    });
  }, []);
  // Per-(stream, thread) agent status. The Streams card aggregates over
  // each stream's threads to render the "working" indicator.
  const [agentStatuses, setAgentStatuses] = useState<Record<string, Record<string, string>>>({});
  const streamId = stream?.id ?? null;

  const refresh = useCallback(async () => {
    if (!streamId) {
      setData(null);
      setLoading(false);
      return;
    }
    try {
      setError(null);
      const head = await vcsHead(streamId);
      const headSha = vcsRevOf(head.revision);
      const [statusSummary, history, remoteBranches, streams] = await Promise.all([
        vcsStatus(streamId).then(countStatus),
        readHistory(headSha, RECENT_LIMIT),
        listRecentRemoteBranches(streamId, 20),
        listStreams(),
      ]);
      const divergence = await readStreamDivergences(streamId, streams);
      const log = { ...history, currentBranch: head.branch };
      const branch = stream?.branch ?? head.branch ?? null;
      const headCommit = log.commits.find((c) => c.id === headSha) ?? null;
      // Find an upstream ref via the remote branches list (best-effort).
      // remoteBranches[].short_name is "<remote>/<branch>" (e.g.
      // "origin/main"). Match the trailing branch name.
      const upstreamRef = branch
        ? remoteBranches.find((r) => {
            const idx = r.short_name.indexOf("/");
            return idx >= 0 && r.short_name.slice(idx + 1) === branch;
          })?.short_name ?? null
        : null;
      let aheadUpstream = 0;
      let behindUpstream = 0;
      if (upstreamRef && head.revision) {
        const counts = await vcsDivergence(streamId, gitRevision(upstreamRef), head.revision);
        aheadUpstream = counts.ahead;
        behindUpstream = counts.behind;
      }
      // Show every other stream (not just sibling worktrees). The
      // dashboard always renders against the currently-viewed stream;
      // each row compares its branch to ours via getAheadBehind, and
      // pulls a fresh uncommitted summary so the user can see at a
      // glance whether each stream has work in flight.
      const otherStreams = streams.filter((s) => s.id !== streamId);
      const streamRows: StreamRow[] = await Promise.all(
        otherStreams.map(async (other) => {
          const uncommitted = await vcsStatus(other.id).then(countStatus).catch(() => null);
          const otherBranch = other.branch || null;
          if (!otherBranch || !branch || otherBranch === branch) {
            return { stream: other, branch: otherBranch, ahead: 0, behind: 0, uncommitted };
          }
          const counts = await vcsDivergence(streamId, gitRevision(branch), gitRevision(otherBranch));
          return { stream: other, branch: otherBranch, ahead: counts.ahead, behind: counts.behind, uncommitted };
        }),
      );
      setData({
        branchHeader: {
          branch,
          headSha: headCommit?.id ?? null,
          headSubject: headCommit?.subject ?? null,
          headDate: headCommit?.time ?? null,
          upstream: upstreamRef,
          aheadUpstream,
          behindUpstream,
        },
        uncommitted: statusSummary,
        recentLog: log,
        streams: streamRows,
        remoteBranches,
        divergence,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [streamId, stream?.branch]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);
  // The commit indexer lands new commits in `v_commit` after a ref moves.
  useRerunOnChange(data?.recentLog.reads ?? NO_READS, () => void refresh());

  // Debounce watcher-driven refreshes: a single `git rebase`/`git merge`
  // can fire .git/refs and workspace events dozens of times in quick
  // succession. Each refresh is 5+ parallel IPC calls — without
  // debouncing, the avalanche locks up the renderer and stalls the
  // post-action refresh awaited by `runConfirmed`.
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scheduleRefresh = useCallback(() => {
    if (refreshTimer.current) clearTimeout(refreshTimer.current);
    refreshTimer.current = setTimeout(() => {
      refreshTimer.current = null;
      void refresh();
    }, 250);
  }, [refresh]);

  useEffect(() => {
    if (!streamId) return;
    const unsubGit = subscribeGitRefsEvents(streamId, scheduleRefresh);
    const unsubWorkspace = subscribeWorkspaceEvents(streamId, scheduleRefresh);
    return () => {
      unsubGit();
      unsubWorkspace();
      if (refreshTimer.current) {
        clearTimeout(refreshTimer.current);
        refreshTimer.current = null;
      }
    };
  }, [streamId, scheduleRefresh]);

  // Other-stream rows (the dashboard's worktrees / siblings list) read
  // uncommitted + ahead/behind for every other stream. Without
  // subscribing to those streams' watcher events, a commit done from
  // inside another stream's tab leaves the dashboard showing stale
  // uncommitted counts. Subscribe to every other-stream id once we
  // know the list, so any change triggers the same debounced refresh.
  const otherStreamIds = useMemo(
    () => (data?.streams ?? []).map((row) => row.stream.id).join(","),
    [data?.streams],
  );
  useEffect(() => {
    if (!otherStreamIds) return;
    const ids = otherStreamIds.split(",").filter(Boolean);
    const unsubs = ids.flatMap((id) => [
      subscribeGitRefsEvents(id, scheduleRefresh),
      subscribeWorkspaceEvents(id, scheduleRefresh),
    ]);
    return () => {
      for (const fn of unsubs) {
        try { fn(); } catch { /* ignore unsubscribe errors */ }
      }
    };
  }, [otherStreamIds, scheduleRefresh]);

  useEffect(() => {
    let cancelled = false;
    void listAgentStatuses().then((entries) => {
      if (cancelled) return;
      const byStream: Record<string, Record<string, string>> = {};
      for (const e of entries) {
        (byStream[e.streamId] ??= {})[e.threadId] = e.status;
      }
      setAgentStatuses(byStream);
    });
    const unsub = subscribeAgentStatus("all", (entry) => {
      setAgentStatuses((prev: Record<string, Record<string, string>>) => ({
        ...prev,
        [entry.streamId]: { ...(prev[entry.streamId] ?? {}), [entry.threadId]: entry.status },
      }));
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, []);

  const streamWorkingFlags = useMemo(() => {
    const out: Record<string, boolean> = {};
    for (const sid of Object.keys(agentStatuses)) {
      const threads = agentStatuses[sid] ?? {};
      out[sid] = Object.values(threads).some((s) => s === "working");
    }
    return out;
  }, [agentStatuses]);

  // Each git action's button asks first when its command's spec does
  // (`SpecConfirm`, tsk898); the run goes confirmed either way — the
  // person confirmed, or the command doesn't ask.
  const runOp = useCallback(
    async (label: string, command: string, action: () => Promise<import("../api.js").GitOpKickoff>) => {
      addPending(label);
      let result: OpOutcome;
      try {
        result = await awaitGitOp(await action());
      } finally {
        removePending(label);
      }
      // A failure surfaces globally (toast + status-bar indicator) via
      // recordOpError — no per-site toast. Refresh either way so any
      // partial progress is reflected.
      if (!result.success) recordOpError(opErrorOf(label, command, result));
      void refresh();
    },
    [refresh, onOpenPage, addPending, removePending],
  );

  const runUnconfirmed = useCallback(
    async (label: string, action: () => Promise<import("../api.js").GitOpKickoff>) => {
      addPending(label);
      let result: OpOutcome;
      try {
        result = await awaitGitOp(await action());
      } finally {
        removePending(label);
      }
      if (!result.success) {
        recordOpError({ label, message: gitOpErrorMessage(result, "error") });
      } else {
        void refresh();
      }
    },
    [refresh, addPending, removePending],
  );

  if (!streamId) {
    return (
      <Page testId="page-git-dashboard" title="Git Dashboard">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  const dashboardTitle = data?.branchHeader.branch
    ? `Git Dashboard: ${data.branchHeader.branch}`
    : "Git Dashboard";

  return (
    <Page testId="page-git-dashboard" title={dashboardTitle}>
      <div
        style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}
        data-ref-kind="git-dashboard"
        data-ref-id="git-dashboard"
      >
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}

        {data ? (
          <>
            <UpstreamCard
              data={data.branchHeader}
              onPush={() =>
                runOp(
                  "Push",
                  "push",
                  () => vcsPush(streamId, undefined, true),
                )
              }
              onPullUpstream={() =>
                runOp(
                  "Pull",
                  "pull",
                  () => vcsPull(streamId, undefined, true),
                )
              }
              onFetch={() => runUnconfirmed("Fetch", () => vcsFetch(streamId))}
              isPending={isPending}
            />

            <UncommittedMiniCard
              summary={data.uncommitted}
              onView={() => onOpenPage(uncommittedChangesRef())}
            />

            <RecentCommitsCard
              streamId={streamId}
              log={data.recentLog}
              onSelectCommit={handleSelectCommit}
              onViewFullHistory={() => onOpenPage(indexRef("git-history"))}
            />

            <StreamsCard
              streamId={streamId}
              rows={data.streams}
              currentBranch={data.branchHeader.branch}
              workingByStreamId={streamWorkingFlags}
              onSelectCommit={handleSelectCommit}
              onMerge={(branch) =>
                runOp(
                  `Merge ${branch} into current`,
                  `merge ${branch}`,
                  () => vcsMerge(streamId, branch, true),
                )
              }
              onRebase={(branch) =>
                runOp(
                  `Rebase current onto ${branch}`,
                  `rebase ${branch}`,
                  () => gitRebase(streamId, branch, true),
                )
              }
              isPending={isPending}
            />

            <MergeReadinessCard
              report={data.divergence}
              currentBranch={data.branchHeader.branch}
              onMerge={(branch) =>
                runOp(
                  `Merge ${branch} into ${data.branchHeader.branch ?? "current"}`,
                  `merge ${branch}`,
                  () => vcsMerge(streamId, branch, true),
                )
              }
              isPending={isPending}
            />

            <RemoteBranchesCard
              streamId={streamId}
              rows={data.remoteBranches}
              onPull={(remote, branch) =>
                runOp(
                  `Pull ${remote}/${branch} into current`,
                  `pull ${remote} ${branch}`,
                  () => vcsPull(streamId, { remote, branch }, true),
                )
              }
              onPush={(remote, branch) =>
                runOp(
                  `Push current → ${remote}/${branch}`,
                  `push ${remote} ${branch}`,
                  () => vcsPush(streamId, { remote, branch }, true),
                )
              }
              isPending={isPending}
            />
          </>
        ) : null}
      </div>
    </Page>
  );
}

function UpstreamCard({
  data,
  onPush,
  onPullUpstream,
  onFetch,
  isPending,
}: {
  data: DashboardData["branchHeader"];
  onPush(): void;
  onPullUpstream(): void;
  onFetch(): void;
  isPending(label: string): boolean;
}) {
  const hasUpstream = !!data.upstream;
  const pushing = isPending("Push");
  const pulling = isPending("Pull");
  const fetching = isPending("Fetch");
  const nothingToPush = data.aheadUpstream === 0;
  const nothingToPull = data.behindUpstream === 0;
  return (
    <Card testId="git-dashboard-upstream" title="Upstream">
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {hasUpstream ? (
          <div style={{ ...subtle, display: "inline-flex", alignItems: "center", gap: 8 }}>
            <span>tracks <code>{data.upstream}</code></span>
            <AheadBehindBadge
              ahead={data.aheadUpstream}
              behind={data.behindUpstream}
              context={data.upstream ?? "upstream"}
            />
          </div>
        ) : (
          <div style={subtle}>No upstream</div>
        )}
        <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
          {hasUpstream ? (
            <>
              <SpecConfirm command="vcs.push" onConfirm={onPush} confirmLabel="Push" testIdPrefix="git-dashboard-push">
                {(run) => (
                  <button
                    type="button"
                    data-testid="git-dashboard-push"
                    onClick={run}
                    disabled={pushing || nothingToPush}
                    style={primaryButton}
                  >
                    {pushing ? "Pushing…" : "Push"}
                  </button>
                )}
              </SpecConfirm>
              <SpecConfirm command="vcs.pull" onConfirm={onPullUpstream} confirmLabel="Pull" testIdPrefix="git-dashboard-pull">
                {(run) => (
                  <button
                    type="button"
                    data-testid="git-dashboard-pull"
                    onClick={run}
                    disabled={pulling || nothingToPull}
                    style={smallButton}
                  >
                    {pulling ? "Pulling…" : "Pull"}
                  </button>
                )}
              </SpecConfirm>
            </>
          ) : null}
          <button
            type="button"
            data-testid="git-dashboard-fetch"
            onClick={onFetch}
            disabled={fetching}
            style={smallButton}
          >
            {fetching ? "Fetching…" : "Fetch"}
          </button>
        </div>
      </div>
    </Card>
  );
}

function UncommittedMiniCard({
  summary,
  onView,
}: {
  summary: StatusCounts | null;
  onView(): void;
}) {
  const total = summary?.total ?? 0;
  return (
    <Card
      testId="git-dashboard-uncommitted-mini"
      title="Uncommitted"
      action={
        <button
          type="button"
          data-testid="git-dashboard-view-uncommitted"
          onClick={onView}
          style={linkButton}
        >
          View uncommitted →
        </button>
      }
    >
      {total === 0 || !summary ? (
        <div style={subtle}>No uncommitted files</div>
      ) : (
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <span style={{ fontSize: "var(--text-sm)" }}>{summary.total} changed</span>
          <FileStatusCountsForSummary summary={summary} testId="git-dashboard-uncommitted-counts" />
        </div>
      )}
    </Card>
  );
}

function useCommitStats(streamId: string, commits: RevisionInfo[]): Map<string, CommitStats> {
  const [stats, setStats] = useState<Map<string, CommitStats>>(new Map());
  const shaKey = commits.map((c) => c.id).join(",");
  useEffect(() => {
    let cancelled = false;
    const shas = commits.map((c) => c.id);
    void Promise.all(
      shas.map(async (sha) => {
        const detail = await vcsRevision(streamId, gitRevision(sha));
        if (!detail) return [sha, null] as const;
        let filesAdded = 0;
        let filesModified = 0;
        let filesDeleted = 0;
        let additions = 0;
        let deletions = 0;
        for (const f of detail.files) {
          if (f.status === "added" || f.status === "untracked") filesAdded += 1;
          else if (f.status === "deleted") filesDeleted += 1;
          else filesModified += 1;
          additions += f.additions ?? 0;
          deletions += f.deletions ?? 0;
        }
        return [sha, { filesAdded, filesModified, filesDeleted, additions, deletions }] as const;
      }),
    ).then((entries) => {
      if (cancelled) return;
      const next = new Map<string, CommitStats>();
      for (const [sha, s] of entries) if (s) next.set(sha, s);
      setStats(next);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamId, shaKey]);
  return stats;
}

function RecentCommitsCard({
  streamId,
  log,
  onSelectCommit,
  onViewFullHistory,
}: {
  streamId: string;
  log: History & { currentBranch: string | null };
  onSelectCommit(sha: string): void;
  onViewFullHistory(): void;
}) {
  const refIndex = useMemo(() => indexRefsBySha(log), [log]);
  const stats = useCommitStats(streamId, log.commits);

  return (
    <Card
      testId="git-dashboard-recent-commits"
      title="Recent Commits"
      action={
        <button
          type="button"
          data-testid="git-dashboard-view-full-history"
          onClick={onViewFullHistory}
          style={linkButton}
        >
          View full history →
        </button>
      }
    >
      {log.commits.length === 0 ? (
        <EmptyState
          compact
          title="No commits yet"
          text="Commits on this branch show up here as they're made."
          prompts={["What's uncommitted right now, and what should go in the first commit?"]}
        />
      ) : (
        <CommitGraphTable
          commits={log.commits}
          branchHeadsBySha={refIndex.branchHeadsBySha}
          tagsBySha={refIndex.tagsBySha}
          currentBranch={log.currentBranch}
          statsBySha={stats}
          onSelect={onSelectCommit}
        />
      )}
    </Card>
  );
}

function StreamsCard({
  streamId,
  rows,
  currentBranch,
  onMerge,
  onRebase,
  onSelectCommit,
  isPending,
  workingByStreamId,
}: {
  streamId: string;
  rows: StreamRow[];
  currentBranch: string | null;
  onMerge(branch: string): void;
  onRebase(branch: string): void;
  onSelectCommit(sha: string): void;
  isPending(label: string): boolean;
  workingByStreamId: Record<string, boolean>;
}) {
  const [expanded, setExpanded] = useState<string | null>(null);
  return (
    <Card testId="git-dashboard-streams" title="Streams">
      {rows.length === 0 ? (
        <div style={muted}>No other streams.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row) => {
            const branchLabel = row.branch ?? "(detached)";
            const mergeLabel = `Merge ${branchLabel} into current`;
            const rebaseLabel = `Rebase current onto ${branchLabel}`;
            const isOpen = expanded === row.stream.id;
            return (
              <div
                key={row.stream.id}
                data-testid="git-dashboard-stream-row"
                style={{ borderBottom: "1px solid var(--border-subtle)" }}
              >
                <div
                  style={{
                    display: "flex",
                    gap: 12,
                    alignItems: "center",
                    padding: "6px 0",
                  }}
                >
                  <button
                    type="button"
                    onClick={() => setExpanded(isOpen ? null : row.stream.id)}
                    style={{
                      ...linkButton,
                      width: 16,
                      fontSize: 16,
                      color: "var(--text-muted)",
                    }}
                    aria-label={isOpen ? "Hide pairwise diff" : "Show pairwise diff"}
                  >
                    {isOpen ? "▾" : "▸"}
                  </button>
                  <div style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "baseline", gap: 6, overflow: "hidden" }}>
                    {workingByStreamId[row.stream.id] ? (
                      <AgentStatusDot status="working" />
                    ) : null}
                    <span style={{ fontWeight: "var(--weight-medium)", flexShrink: 0 }}>{row.stream.title}</span>
                    <span style={{ ...subtle, flexShrink: 0 }}>·</span>
                    <span style={{ ...subtle, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {branchLabel}
                    </span>
                  </div>
                  <UncommittedSummaryInline summary={row.uncommitted} />
                  <AheadBehindBadge
                    ahead={row.ahead}
                    behind={row.behind}
                    context={`${currentBranch ?? "current"}`}
                  />
                  {row.branch ? (
                    <MergeRebaseSplitButton
                      streamId={streamId}
                      branch={row.branch}
                      onMerge={onMerge}
                      onRebase={onRebase}
                      mergePending={isPending(mergeLabel)}
                      rebasePending={isPending(rebaseLabel)}
                      ahead={row.ahead}
                    />
                  ) : null}
                </div>
                {isOpen && row.branch ? (
                  <PairwiseDiffPane
                    streamId={streamId}
                    siblingBranch={row.branch}
                    currentBranch={currentBranch}
                    onSelectCommit={onSelectCommit}
                  />
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </Card>
  );
}

function UncommittedSummaryInline({ summary }: { summary: StatusCounts | null }) {
  if (!summary || summary.total === 0) {
    return (
      <span
        style={{ ...subtle, fontStyle: "italic" }}
        title="Working tree is clean — no uncommitted changes."
      >
        clean
      </span>
    );
  }
  return <FileStatusCountsForSummary summary={summary} testId="git-dashboard-stream-uncommitted" />;
}

function AheadBehindBadge({
  ahead,
  behind,
  context,
  testId,
}: {
  ahead: number;
  behind: number;
  /** Short noun for the comparand, e.g. "main" or "origin/main" — interpolated into the tooltip. */
  context: string;
  testId?: string;
}) {
  const title =
    `↑ ${ahead} outgoing — commits in this branch not yet in ${context}\n` +
    `↓ ${behind} incoming — commits in ${context} not yet in this branch`;
  return (
    <span
      data-testid={testId}
      title={title}
      style={{ ...subtle, cursor: "help", whiteSpace: "nowrap" }}
    >
      ↑{ahead} ↓{behind}
    </span>
  );
}


const READINESS_STYLE: Record<
  StreamDivergenceRow["readiness"],
  { label: string; color: string; bg: string }
> = {
  clean: { label: "Clean to merge", color: "#3fb950", bg: "rgba(63,185,80,0.12)" },
  conflict: { label: "Will conflict", color: "#d29922", bg: "rgba(210,153,34,0.12)" },
  already_integrated: { label: "Integrated", color: "var(--text-muted)", bg: "transparent" },
};

function ReadinessBadge({ readiness }: { readiness: StreamDivergenceRow["readiness"] }) {
  const s = READINESS_STYLE[readiness];
  return (
    <span
      data-testid="git-dashboard-divergence-readiness"
      data-readiness={readiness}
      style={{
        display: "inline-block",
        padding: "1px 8px",
        borderRadius: 999,
        fontSize: "var(--text-xs)",
        fontWeight: "var(--weight-medium)",
        color: s.color,
        background: s.bg,
        whiteSpace: "nowrap",
      }}
    >
      {s.label}
    </span>
  );
}

/// Cross-stream divergence vs the integration branch. Each row shows a
/// stream's ahead/behind + merge-readiness; a clean stream can be merged
/// in one click, but only while you're viewing the integration branch
/// itself (the merge runs into the current stream's branch).
function MergeReadinessCard({
  report,
  currentBranch,
  onMerge,
  isPending,
}: {
  report: StreamDivergenceReport;
  currentBranch: string | null;
  onMerge(branch: string): void;
  isPending(label: string): boolean;
}) {
  // Only the streams that actually diverge from the base are worth
  // listing — the integration branch itself and fully-merged streams
  // would just be noise.
  const rows = report.rows.filter((r) => r.branch !== report.base && r.ahead > 0);
  const onBase = currentBranch === report.base;
  return (
    <Card
      testId="git-dashboard-divergence"
      title={`Merge readiness vs ${report.base}`}
    >
      {rows.length === 0 ? (
        <div style={subtle}>Every stream is integrated with {report.base}.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {rows.map((row) => {
            const mergeLabel = `Merge ${row.branch} into ${currentBranch ?? "current"}`;
            const canMerge = onBase && row.readiness === "clean";
            return (
              <div
                key={row.streamId}
                data-testid="git-dashboard-divergence-row"
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 4,
                  padding: "8px 0",
                  borderBottom: "1px solid var(--border-subtle)",
                }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                  <span style={{ fontWeight: "var(--weight-medium)" }}>{row.title}</span>
                  <code style={{ fontSize: "var(--text-xs)" }}>{row.branch}</code>
                  <AheadBehindBadge
                    ahead={row.ahead}
                    behind={row.behind}
                    context={report.base}
                    testId="git-dashboard-divergence-aheadbehind"
                  />
                  <ReadinessBadge readiness={row.readiness} />
                  <span style={{ flex: 1 }} />
                  {canMerge ? (
                    <SpecConfirm
                      command="vcs.merge"
                      onConfirm={() => onMerge(row.branch)}
                      confirmLabel="Merge"
                      testIdPrefix="git-dashboard-divergence-merge"
                    >
                      {(run) => (
                        <button
                          type="button"
                          data-testid="git-dashboard-divergence-merge"
                          onClick={run}
                          disabled={isPending(mergeLabel)}
                          style={primaryButton}
                        >
                          {isPending(mergeLabel) ? "Merging…" : `Merge into ${report.base}`}
                        </button>
                      )}
                    </SpecConfirm>
                  ) : null}
                </div>
                {row.readiness === "conflict" ? (
                  <div style={subtle} data-testid="git-dashboard-divergence-overlap">
                    Overlapping {row.overlappingFiles.length === 1 ? "file" : "files"}:{" "}
                    {row.overlappingFiles.slice(0, 8).map((f, i) => (
                      <span key={f}>
                        {i > 0 ? ", " : ""}
                        <code>{f}</code>
                      </span>
                    ))}
                    {row.overlappingFiles.length > 8
                      ? ` +${row.overlappingFiles.length - 8} more`
                      : ""}
                  </div>
                ) : null}
              </div>
            );
          })}
          {!onBase ? (
            <div style={{ ...subtle, paddingTop: 8 }}>
              Switch to the <code>{report.base}</code> stream to merge a clean stream in.
            </div>
          ) : null}
        </div>
      )}
    </Card>
  );
}

type MergeRebaseMode = "merge" | "rebase";

const MERGE_MODE_PREFIX = "oxplow.gitDashboard.mergeMode";

function mergeModeKey(streamId: string, branch: string): string {
  return `${MERGE_MODE_PREFIX}.${streamId}.${branch}`;
}

function readMergeMode(streamId: string, branch: string): MergeRebaseMode {
  try {
    const v = window.localStorage.getItem(mergeModeKey(streamId, branch));
    return v === "rebase" ? "rebase" : "merge";
  } catch {
    return "merge";
  }
}

function writeMergeMode(streamId: string, branch: string, mode: MergeRebaseMode): void {
  try {
    window.localStorage.setItem(mergeModeKey(streamId, branch), mode);
  } catch {
    // ignore storage errors
  }
}

function MergeRebaseSplitButton({
  streamId,
  branch,
  onMerge,
  onRebase,
  mergePending,
  rebasePending,
  ahead,
}: {
  streamId: string;
  branch: string;
  onMerge(branch: string): void;
  onRebase(branch: string): void;
  mergePending: boolean;
  rebasePending: boolean;
  /** Number of commits in `branch` not in the current branch. When 0,
   *  there is nothing to merge or rebase, so the button is disabled. */
  ahead: number;
}) {
  const [mode, setMode] = useState<MergeRebaseMode>(() => readMergeMode(streamId, branch));
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    setMode(readMergeMode(streamId, branch));
  }, [streamId, branch]);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = () => setMenuOpen(false);
    window.addEventListener("click", handler);
    return () => window.removeEventListener("click", handler);
  }, [menuOpen]);

  const choose = (next: MergeRebaseMode) => {
    setMode(next);
    writeMergeMode(streamId, branch, next);
    setMenuOpen(false);
  };

  const pending = mode === "merge" ? mergePending : rebasePending;
  const nothingToDo = ahead === 0;
  const disabled = pending || nothingToDo;
  const idleLabel = mode === "merge" ? "Merge In" : "Rebase Onto";
  const busyLabel = mode === "merge" ? "Merging…" : "Rebasing…";
  const primaryTitle = nothingToDo
    ? `${branch} has no commits not already in the current branch — nothing to ${mode === "merge" ? "merge" : "rebase"}.`
    : undefined;
  const onPrimary = () => (mode === "merge" ? onMerge(branch) : onRebase(branch));

  return (
    <div style={{ position: "relative", display: "inline-flex" }}>
      <SpecConfirm
        command={mode === "merge" ? "vcs.merge" : "git.rebase"}
        onConfirm={onPrimary}
        confirmLabel={mode === "merge" ? "Merge" : "Rebase"}
        testIdPrefix="git-dashboard-stream-merge-rebase"
      >
        {(run) => (
          <button
            type="button"
            data-testid="git-dashboard-stream-merge-rebase"
            data-mode={mode}
            onClick={run}
            disabled={disabled}
            title={primaryTitle}
            style={{ ...smallButton, borderTopRightRadius: 0, borderBottomRightRadius: 0, borderRight: "none" }}
          >
            {pending ? busyLabel : idleLabel}
          </button>
        )}
      </SpecConfirm>
      <button
        type="button"
        aria-label="Choose merge or rebase"
        data-testid="git-dashboard-stream-merge-rebase-menu"
        onClick={(e) => {
          e.stopPropagation();
          setMenuOpen((v) => !v);
        }}
        disabled={disabled}
        title={primaryTitle}
        style={{
          ...smallButton,
          padding: "2px 6px",
          borderTopLeftRadius: 0,
          borderBottomLeftRadius: 0,
        }}
      >
        ▾
      </button>
      {menuOpen ? (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            position: "absolute",
            top: "100%",
            right: 0,
            marginTop: 2,
            background: "var(--surface-card)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 4,
            boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
            zIndex: 10,
            minWidth: 140,
            display: "flex",
            flexDirection: "column",
          }}
        >
          <button
            type="button"
            onClick={() => choose("merge")}
            style={menuItem(mode === "merge")}
          >
            Merge In
          </button>
          <button
            type="button"
            onClick={() => choose("rebase")}
            style={menuItem(mode === "rebase")}
          >
            Rebase Onto
          </button>
        </div>
      ) : null}
    </div>
  );
}

function menuItem(active: boolean): React.CSSProperties {
  return {
    padding: "6px 10px",
    background: active ? "var(--surface-tab-active, var(--surface-card))" : "transparent",
    color: "var(--text-primary)",
    border: "none",
    borderBottom: "1px solid var(--border-subtle)",
    textAlign: "left",
    fontSize: "var(--text-xs)",
    cursor: "pointer",
    fontWeight: active ? 600 : 400,
  };
}

function PairwiseDiffPane({
  streamId,
  siblingBranch,
  currentBranch,
  onSelectCommit,
}: {
  streamId: string;
  siblingBranch: string;
  currentBranch: string | null;
  onSelectCommit(sha: string): void;
}) {
  const target = currentBranch && currentBranch !== siblingBranch ? currentBranch : "";
  const [commits, setCommits] = useState<RevisionInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const stats = useCommitStats(streamId, commits);

  useEffect(() => {
    if (!target) {
      setCommits([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void vcsRevisionsBetween(streamId, gitRevision(target), gitRevision(siblingBranch), 20)
      .then((result) => {
        if (!cancelled) setCommits(result);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [streamId, siblingBranch, target]);

  if (!target) {
    return (
      <div style={{ ...subtle, padding: "4px 0 8px 26px" }}>
        {currentBranch
          ? `Same branch as the current stream (${currentBranch}); nothing to compare.`
          : "Current stream is detached; nothing to compare against."}
      </div>
    );
  }
  return (
    <div
      data-testid="git-dashboard-worktree-pairwise"
      style={{ padding: "4px 0 8px 26px", display: "flex", flexDirection: "column", gap: 6 }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span style={subtle}>
          Commits in <code>{siblingBranch}</code> not in <code>{target}</code>
        </span>
      </div>
      {loading ? (
        <div style={subtle}>Loading…</div>
      ) : commits.length === 0 ? (
        <div style={subtle}>No commits ahead.</div>
      ) : (
        <CommitGraphTable
          commits={commits}
          branchHeadsBySha={EMPTY_REF_MAP}
          tagsBySha={EMPTY_REF_MAP}
          currentBranch={null}
          statsBySha={stats}
          onSelect={onSelectCommit}
        />
      )}
    </div>
  );
}

const EMPTY_REF_MAP: Map<string, string[]> = new Map();

function RemoteBranchesCard({
  streamId,
  rows,
  onPull,
  onPush,
  isPending,
}: {
  streamId: string;
  rows: RemoteBranchEntry[];
  onPull(remote: string, branch: string): void;
  onPush(remote: string, branch: string): void;
  isPending(label: string): boolean;
}) {
  const [counts, setCounts] = useState<Record<string, { ahead: number; behind: number }>>({});

  useEffect(() => {
    let cancelled = false;
    void Promise.all(
      rows.map(async (row) => {
        const res = await vcsDivergence(streamId, gitRevision(row.short_name), gitRevision("HEAD"));
        return [row.short_name, res] as const;
      }),
    ).then((entries) => {
      if (cancelled) return;
      const out: Record<string, { ahead: number; behind: number }> = {};
      for (const [k, v] of entries) out[k] = { ahead: v.ahead, behind: v.behind };
      setCounts(out);
    });
    return () => {
      cancelled = true;
    };
  }, [streamId, rows]);

  return (
    <Card testId="git-dashboard-remote-branches" title="Recent Remote Branches">
      {rows.length === 0 ? (
        <div style={muted}>No remote branches.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row) => {
            const pullLabel = `Pull ${row.short_name} into current`;
            const pushLabel = `Push current → ${row.short_name}`;
            const c = counts[row.short_name];
            return (
              <div
                key={row.short_name}
                data-testid="git-dashboard-remote-row"
                style={{
                  display: "flex",
                  gap: 12,
                  alignItems: "center",
                  padding: "6px 0",
                  borderBottom: "1px solid var(--border-subtle)",
                }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontWeight: "var(--weight-medium)" }}>{row.short_name}</div>
                  <div style={{ ...subtle, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {row.last_commit_subject} · {row.last_commit_at} · {formatDate(row.last_commit_at)}
                  </div>
                </div>
                <AheadBehindBadge
                  ahead={c?.ahead ?? 0}
                  behind={c?.behind ?? 0}
                  context={row.short_name}
                />
                <SpecConfirm
                  command="vcs.pull"
                  onConfirm={() => {
                    const [remote, ...rest] = row.short_name.split("/");
                    onPull(remote, rest.join("/"));
                  }}
                  confirmLabel="Pull"
                  testIdPrefix={`git-dashboard-remote-pull-${row.short_name}`}
                >
                  {(run) => (
                    <button
                      type="button"
                      onClick={run}
                      disabled={isPending(pullLabel) || (c?.behind ?? 0) === 0}
                      title={
                        (c?.behind ?? 0) === 0
                          ? `${row.short_name} has no commits not already in current — nothing to pull.`
                          : undefined
                      }
                      style={smallButton}
                    >
                      {isPending(pullLabel) ? "Pulling…" : "Pull into"}
                    </button>
                  )}
                </SpecConfirm>
                <SpecConfirm
                  command="vcs.push"
                  onConfirm={() => {
                    const [remote, ...rest] = row.short_name.split("/");
                    onPush(remote, rest.join("/"));
                  }}
                  confirmLabel="Push"
                  testIdPrefix={`git-dashboard-remote-push-${row.short_name}`}
                >
                  {(run) => (
                    <button
                      type="button"
                      onClick={run}
                      disabled={isPending(pushLabel) || (c?.ahead ?? 0) === 0}
                      title={
                        (c?.ahead ?? 0) === 0
                          ? `Current has no commits not already in ${row.short_name} — nothing to push.`
                          : undefined
                      }
                      style={smallButton}
                    >
                      {isPending(pushLabel) ? "Pushing…" : "Push to"}
                    </button>
                  )}
                </SpecConfirm>
              </div>
            );
          })}
        </div>
      )}
    </Card>
  );
}

function formatDate(input: string | number | null | undefined): string {
  if (!input && input !== 0) return "";
  try {
    // Bindings ship Unix-seconds numbers; legacy callers pass ISO strings.
    const d =
      typeof input === "number" ? new Date(input * 1000) : new Date(input);
    return d.toLocaleDateString();
  } catch {
    return String(input);
  }
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const primaryButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-action, #2563eb)",
  color: "var(--text-inverse, white)",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
};
const smallButton: React.CSSProperties = {
  padding: "2px 8px",
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};
const linkButton: React.CSSProperties = cardLinkButton;

