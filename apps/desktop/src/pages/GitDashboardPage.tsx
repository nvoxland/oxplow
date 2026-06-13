import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GitLogCommit, GitLogResult, GitOpResult, RemoteBranchEntry, Stream, StreamDivergenceReport, StreamDivergenceRow, WorkspaceStatusSummary } from "../api.js";
import {
  getAheadBehind,
  listStreamDivergences,
  getCommitDetail,
  getCommitsAheadOf,
  getGitLog,
  gitFetch,
  gitMergeInto,
  gitRebaseOnto,
  gitPull,
  gitPullRemoteIntoCurrent,
  gitPush,
  gitPushCurrentTo,
  listAgentStatuses,
  listRecentRemoteBranches,
  listStreams,
  getWorkspaceStatusSummary,
  subscribeAgentStatus,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
} from "../api.js";
import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, indexRef, opErrorRef, uncommittedChangesRef } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { showToast } from "../components/toastStore.js";
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
  uncommitted: WorkspaceStatusSummary | null;
  recentLog: GitLogResult;
  streams: StreamRow[];
  remoteBranches: RemoteBranchEntry[];
  divergence: StreamDivergenceReport;
}

interface StreamRow {
  stream: Stream;
  branch: string | null;
  ahead: number;
  behind: number;
  uncommitted: WorkspaceStatusSummary | null;
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
      const [statusSummary, log, remoteBranches, streams, divergence] = await Promise.all([
        getWorkspaceStatusSummary(streamId),
        getGitLog(streamId, { limit: RECENT_LIMIT, all: false }),
        listRecentRemoteBranches(streamId, 20),
        listStreams(),
        listStreamDivergences(),
      ]);
      const branch = stream?.branch ?? log.currentBranch ?? null;
      const headCommit = log.commits[0] ?? null;
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
      if (upstreamRef) {
        const counts = await getAheadBehind(streamId, upstreamRef);
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
          const uncommitted = await getWorkspaceStatusSummary(other.id).catch(() => null);
          const otherBranch = other.branch || null;
          if (!otherBranch || !branch || otherBranch === branch) {
            return { stream: other, branch: otherBranch, ahead: 0, behind: 0, uncommitted };
          }
          const counts = await getAheadBehind(streamId, branch, otherBranch);
          return { stream: other, branch: otherBranch, ahead: counts.ahead, behind: counts.behind, uncommitted };
        }),
      );
      setData({
        branchHeader: {
          branch,
          headSha: headCommit?.sha ?? null,
          headSubject: headCommit?.subject ?? null,
          headDate: headCommit?.timestamp_secs ?? null,
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

  const runOp = useCallback(
    async (
      label: string,
      command: string,
      action: () => Promise<import("../api.js").GitOpKickoff>,
      opts?: { confirm?: boolean },
    ) => {
      if (opts?.confirm) {
        const ok = window.confirm(`${label}\n\nWill run:\n  ${command}\n\nProceed?`);
        if (!ok) return;
      }
      addPending(label);
      let task: import("../api.js").BackgroundTask | null = null;
      try {
        const { awaitDone } = await action();
        task = await awaitDone;
      } finally {
        removePending(label);
      }
      const result = task?.result as GitOpResult | undefined;
      if (!result || !result.success) {
        const errorId = recordOpError({
          label,
          command,
          stderr: result?.stderr ?? task?.error ?? "",
          stdout: result?.stdout ?? "",
          exitCode: result?.status ?? null,
          args: undefined,
          durationMs: undefined,
          signal: null,
          blankFailure:
            !result || (!result.stderr && !result.stdout && result.status == null),
        });
        // Surface the failure as a toast that lets the user open the
        // detail page on demand. Auto-navigating away switched the
        // active tab, which read as the dashboard "closing" — and the
        // false-failure case (race on awaitDone after a successful git
        // op) made that especially confusing. Refresh either way so any
        // partial progress (e.g. fast-forward that landed before a
        // post-step failed) is reflected.
        showToast({
          message: `${label} failed`,
          actionLabel: "Show details",
          onUndo: () => onOpenPage(opErrorRef(errorId), { newTab: true }),
        });
        void refresh();
      } else {
        void refresh();
      }
    },
    [refresh, onOpenPage, addPending, removePending],
  );

  const runUnconfirmed = useCallback(
    async (label: string, action: () => Promise<import("../api.js").GitOpKickoff>) => {
      addPending(label);
      let task: import("../api.js").BackgroundTask | null = null;
      try {
        const { awaitDone } = await action();
        task = await awaitDone;
      } finally {
        removePending(label);
      }
      const result = task?.result as GitOpResult | undefined;
      if (!result || !result.success) {
        window.alert(`${label} failed:\n${result?.stderr || task?.error || "git error"}`);
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
                  `git push${data.branchHeader.branch ? ` origin ${data.branchHeader.branch}` : ""}`,
                  () => gitPush(streamId),
                  { confirm: true },
                )
              }
              onPullUpstream={() =>
                runOp(
                  "Pull",
                  `git pull${data.branchHeader.branch ? ` origin ${data.branchHeader.branch}` : ""}`,
                  () => gitPull(streamId),
                )
              }
              onFetch={() => runUnconfirmed("Fetch", () => gitFetch(streamId))}
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
                  `git merge ${branch}`,
                  () => gitMergeInto(streamId, branch),
                )
              }
              onRebase={(branch) =>
                runOp(
                  `Rebase current onto ${branch}`,
                  `git rebase ${branch}`,
                  () => gitRebaseOnto(streamId, branch),
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
                  `git merge ${branch}`,
                  () => gitMergeInto(streamId, branch),
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
                  `git fetch ${remote} ${branch} && git merge ${remote}/${branch}`,
                  () => gitPullRemoteIntoCurrent(streamId, remote, branch),
                )
              }
              onPush={(remote, branch) =>
                runOp(
                  `Push current → ${remote}/${branch}`,
                  `git push ${remote} HEAD:refs/heads/${branch}`,
                  () => gitPushCurrentTo(streamId, remote, branch),
                  { confirm: true },
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
              <button
                type="button"
                data-testid="git-dashboard-push"
                onClick={onPush}
                disabled={pushing || nothingToPush}
                style={primaryButton}
              >
                {pushing ? "Pushing…" : "Push"}
              </button>
              <button
                type="button"
                data-testid="git-dashboard-pull"
                onClick={onPullUpstream}
                disabled={pulling || nothingToPull}
                style={smallButton}
              >
                {pulling ? "Pulling…" : "Pull"}
              </button>
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
  summary: WorkspaceStatusSummary | null;
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

function useCommitStats(streamId: string, commits: GitLogCommit[]): Map<string, CommitStats> {
  const [stats, setStats] = useState<Map<string, CommitStats>>(new Map());
  const shaKey = commits.map((c) => c.sha).join(",");
  useEffect(() => {
    let cancelled = false;
    const shas = commits.map((c) => c.sha);
    void Promise.all(
      shas.map(async (sha) => {
        const detail = await getCommitDetail(streamId, sha);
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
  log: GitLogResult;
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
        <div style={muted}>No commits yet.</div>
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

function UncommittedSummaryInline({ summary }: { summary: WorkspaceStatusSummary | null }) {
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
  "already-integrated": { label: "Integrated", color: "var(--text-muted)", bg: "transparent" },
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
                key={row.stream_id}
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
                    <button
                      type="button"
                      data-testid="git-dashboard-divergence-merge"
                      onClick={() => onMerge(row.branch)}
                      disabled={isPending(mergeLabel)}
                      style={primaryButton}
                    >
                      {isPending(mergeLabel) ? "Merging…" : `Merge into ${report.base}`}
                    </button>
                  ) : null}
                </div>
                {row.readiness === "conflict" ? (
                  <div style={subtle} data-testid="git-dashboard-divergence-overlap">
                    Overlapping {row.overlapping_files.length === 1 ? "file" : "files"}:{" "}
                    {row.overlapping_files.slice(0, 8).map((f, i) => (
                      <span key={f}>
                        {i > 0 ? ", " : ""}
                        <code>{f}</code>
                      </span>
                    ))}
                    {row.overlapping_files.length > 8
                      ? ` +${row.overlapping_files.length - 8} more`
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
      <button
        type="button"
        data-testid="git-dashboard-stream-merge-rebase"
        data-mode={mode}
        onClick={onPrimary}
        disabled={disabled}
        title={primaryTitle}
        style={{ ...smallButton, borderTopRightRadius: 0, borderBottomRightRadius: 0, borderRight: "none" }}
      >
        {pending ? busyLabel : idleLabel}
      </button>
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
  const [commits, setCommits] = useState<GitLogCommit[]>([]);
  const [loading, setLoading] = useState(false);
  const stats = useCommitStats(streamId, commits);

  useEffect(() => {
    if (!target) {
      setCommits([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void getCommitsAheadOf(streamId, target, siblingBranch, 20)
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
        const res = await getAheadBehind(streamId, row.short_name);
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
                <button
                  type="button"
                  onClick={() => {
                    const [remote, ...rest] = row.short_name.split("/");
                    onPull(remote, rest.join("/"));
                  }}
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
                <button
                  type="button"
                  onClick={() => {
                    const [remote, ...rest] = row.short_name.split("/");
                    onPush(remote, rest.join("/"));
                  }}
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

