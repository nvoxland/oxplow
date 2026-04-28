import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GitLogCommit, GitLogResult, GitOpResult, GitWorktreeEntry, RemoteBranchEntry, Stream, WorkspaceStatusSummary } from "../api.js";
import {
  getAheadBehind,
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
  listSiblingWorktrees,
  listStreams,
  listWorkspaceFiles,
  subscribeAgentStatus,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
} from "../api.js";
import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, opErrorRef, uncommittedChangesRef } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
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
    headDate: string | null;
    upstream: string | null;
    aheadUpstream: number;
    behindUpstream: number;
  };
  uncommitted: WorkspaceStatusSummary | null;
  recentLog: GitLogResult;
  streams: StreamWorktreeRow[];
  remoteBranches: RemoteBranchEntry[];
}

interface StreamWorktreeRow {
  worktree: GitWorktreeEntry;
  stream: Stream;
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
      const [filesResult, log, worktrees, remoteBranches, streams] = await Promise.all([
        listWorkspaceFiles(streamId),
        getGitLog(streamId, { limit: RECENT_LIMIT, all: false }),
        listSiblingWorktrees(streamId),
        listRecentRemoteBranches(streamId, 20),
        listStreams(),
      ]);
      const branch = stream?.branch ?? log.currentBranch ?? null;
      const headCommit = log.commits[0] ?? null;
      // Find an upstream ref via the remote branches list (best-effort).
      const upstreamRef = branch
        ? remoteBranches.find((r) => r.branch === branch)?.shortName ?? null
        : null;
      let aheadUpstream = 0;
      let behindUpstream = 0;
      if (upstreamRef) {
        const counts = await getAheadBehind(streamId, upstreamRef);
        aheadUpstream = counts.ahead;
        behindUpstream = counts.behind;
      }
      // Only surface worktrees that back a known stream — the dashboard
      // labels rows by the stream's title rather than the worktree path.
      const streamByWorktreePath = new Map(streams.map((s) => [s.worktree_path, s]));
      const streamWorktrees = worktrees.flatMap((wt) => {
        const match = streamByWorktreePath.get(wt.path);
        return match ? [{ wt, stream: match }] : [];
      });
      // Each sibling row compares against the currently-viewed stream's
      // branch — the dashboard is always rendered for one stream, and
      // "ahead/behind vs. self" is the only axis that's meaningful here.
      const streamRows: StreamWorktreeRow[] = await Promise.all(
        streamWorktrees.map(async ({ wt, stream: matchedStream }) => {
          const uncommitted = await listWorkspaceFiles(matchedStream.id)
            .then((r) => r.summary)
            .catch(() => null);
          if (!wt.branch || !branch || wt.branch === branch) {
            return { worktree: wt, stream: matchedStream, ahead: 0, behind: 0, uncommitted };
          }
          const counts = await getAheadBehind(streamId, branch, wt.branch);
          return { worktree: wt, stream: matchedStream, ahead: counts.ahead, behind: counts.behind, uncommitted };
        }),
      );
      setData({
        branchHeader: {
          branch,
          headSha: headCommit?.sha ?? null,
          headSubject: headCommit?.commit.message ?? null,
          headDate: headCommit?.commit.author.date ?? null,
          upstream: upstreamRef,
          aheadUpstream,
          behindUpstream,
        },
        uncommitted: filesResult.summary,
        recentLog: log,
        streams: streamRows,
        remoteBranches,
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

  const runConfirmed = useCallback(
    async (label: string, command: string, action: () => Promise<import("../api.js").GitOpKickoff>) => {
      const ok = window.confirm(`${label}\n\nWill run:\n  ${command}\n\nProceed?`);
      if (!ok) return;
      addPending(label);
      let task: import("../api.js").BackgroundTask | null = null;
      try {
        const { awaitDone } = await action();
        task = await awaitDone;
      } finally {
        removePending(label);
      }
      const result = task?.result as GitOpResult | undefined;
      if (!result || !result.ok) {
        const errorId = recordOpError({
          label,
          command,
          stderr: result?.stderr ?? task?.error ?? "",
          stdout: result?.stdout ?? "",
          exitCode: result?.exitCode ?? null,
        });
        onOpenPage(opErrorRef(errorId), { newTab: true });
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
      if (!result || !result.ok) {
        window.alert(`${label} failed:\n${result?.stderr || task?.error || "git error"}`);
      } else {
        void refresh();
      }
    },
    [refresh, addPending, removePending],
  );

  if (!streamId) {
    return (
      <Page testId="page-git-dashboard" title="Git dashboard">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  const dashboardTitle = data?.branchHeader.branch
    ? `Git dashboard: ${data.branchHeader.branch}`
    : "Git dashboard";

  return (
    <Page testId="page-git-dashboard" title={dashboardTitle}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}

        {data ? (
          <>
            <UpstreamCard
              data={data.branchHeader}
              onPush={() =>
                runConfirmed(
                  "Push",
                  `git push${data.branchHeader.branch ? ` origin ${data.branchHeader.branch}` : ""}`,
                  () => gitPush(streamId),
                )
              }
              onPullUpstream={() =>
                runConfirmed(
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
              onViewFullHistory={() => onRevealCommit(data.recentLog.commits[0]?.sha ?? "")}
            />

            <StreamsCard
              streamId={streamId}
              rows={data.streams}
              currentBranch={data.branchHeader.branch}
              workingByStreamId={streamWorkingFlags}
              onSelectCommit={handleSelectCommit}
              onMerge={(branch) =>
                runConfirmed(
                  `Merge ${branch} into current`,
                  `git merge ${branch}`,
                  () => gitMergeInto(streamId, branch),
                )
              }
              onRebase={(branch) =>
                runConfirmed(
                  `Rebase current onto ${branch}`,
                  `git rebase ${branch}`,
                  () => gitRebaseOnto(streamId, branch),
                )
              }
              isPending={isPending}
            />

            <RemoteBranchesCard
              streamId={streamId}
              rows={data.remoteBranches}
              onPull={(remote, branch) =>
                runConfirmed(
                  `Pull ${remote}/${branch} into current`,
                  `git fetch ${remote} ${branch} && git merge ${remote}/${branch}`,
                  () => gitPullRemoteIntoCurrent(streamId, remote, branch),
                )
              }
              onPush={(remote, branch) =>
                runConfirmed(
                  `Push current → ${remote}/${branch}`,
                  `git push ${remote} HEAD:refs/heads/${branch}`,
                  () => gitPushCurrentTo(streamId, remote, branch),
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
          <span style={{ fontSize: 13 }}>{summary.total} changed</span>
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
      title="Recent commits"
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
  rows: StreamWorktreeRow[];
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
        <div style={muted}>No sibling streams.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row) => {
            const branch = row.worktree.branch ?? "(detached)";
            const mergeLabel = `Merge ${branch} into current`;
            const rebaseLabel = `Rebase current onto ${branch}`;
            const isOpen = expanded === row.worktree.path;
            return (
              <div
                key={row.worktree.path}
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
                    onClick={() => setExpanded(isOpen ? null : row.worktree.path)}
                    style={{
                      ...linkButton,
                      width: 14,
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
                    <span style={{ fontWeight: 500, flexShrink: 0 }}>{row.stream.title}</span>
                    <span style={{ ...subtle, flexShrink: 0 }}>·</span>
                    <span style={{ ...subtle, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {branch}
                    </span>
                  </div>
                  <UncommittedSummaryInline summary={row.uncommitted} />
                  <AheadBehindBadge
                    ahead={row.ahead}
                    behind={row.behind}
                    context={row.worktree.isMain || row.worktree.branch === mainBranchOf(rows) ? "its upstream" : "the main repo branch"}
                  />
                  {row.worktree.branch ? (
                    <MergeRebaseSplitButton
                      streamId={streamId}
                      branch={row.worktree.branch}
                      onMerge={onMerge}
                      onRebase={onRebase}
                      mergePending={isPending(mergeLabel)}
                      rebasePending={isPending(rebaseLabel)}
                      ahead={row.ahead}
                    />
                  ) : null}
                </div>
                {isOpen && row.worktree.branch ? (
                  <PairwiseDiffPane
                    streamId={streamId}
                    siblingBranch={row.worktree.branch}
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
    fontSize: 12,
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

function mainBranchOf(rows: StreamWorktreeRow[]): string | null {
  return rows.find((r) => r.worktree.isMain)?.worktree.branch ?? null;
}

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
        const res = await getAheadBehind(streamId, row.shortName);
        return [row.shortName, res] as const;
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
    <Card testId="git-dashboard-remote-branches" title="Recent remote branches">
      {rows.length === 0 ? (
        <div style={muted}>No remote branches.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row) => {
            const pullLabel = `Pull ${row.shortName} into current`;
            const pushLabel = `Push current → ${row.shortName}`;
            const c = counts[row.shortName];
            return (
              <div
                key={row.shortName}
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
                  <div style={{ fontWeight: 500 }}>{row.shortName}</div>
                  <div style={{ ...subtle, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {row.lastCommitSubject} · {row.lastCommitAuthor} · {formatDate(row.lastCommitDate)}
                  </div>
                </div>
                <AheadBehindBadge
                  ahead={c?.ahead ?? 0}
                  behind={c?.behind ?? 0}
                  context={row.shortName}
                />
                <button
                  type="button"
                  onClick={() => onPull(row.remote, row.branch)}
                  disabled={isPending(pullLabel)}
                  style={smallButton}
                >
                  {isPending(pullLabel) ? "Pulling…" : "Pull into"}
                </button>
                <button
                  type="button"
                  onClick={() => onPush(row.remote, row.branch)}
                  disabled={isPending(pushLabel)}
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

function formatDate(iso: string | null | undefined): string {
  if (!iso) return "";
  try {
    const d = new Date(iso);
    return d.toLocaleDateString();
  } catch {
    return iso;
  }
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
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
  fontSize: 12,
  cursor: "pointer",
};
const linkButton: React.CSSProperties = cardLinkButton;

