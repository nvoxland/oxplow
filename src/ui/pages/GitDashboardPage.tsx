import { useCallback, useEffect, useMemo, useState } from "react";
import type { GitLogCommit, GitLogResult, GitOpResult, GitWorktreeEntry, RemoteBranchEntry, Stream, WorkspaceStatusSummary } from "../api.js";
import {
  getAheadBehind,
  getCommitsAheadOf,
  getGitLog,
  gitMergeInto,
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
import { gitCommitRef, uncommittedChangesRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { CommitGraphTable, indexRefsBySha } from "../components/History/CommitGraphTable.js";

export interface GitDashboardPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
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
}

const RECENT_LIMIT = 5;
const WORKTREE_BASE = "main";

export function GitDashboardPage({ stream, onOpenPage, onRevealCommit }: GitDashboardPageProps) {
  const nav = useOptionalPageNavigation();
  const handleSelectCommit = (sha: string) => {
    if (nav) nav.navigate(gitCommitRef(sha));
    else onRevealCommit(sha);
  };
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<string | null>(null);
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
      const streamRows: StreamWorktreeRow[] = await Promise.all(
        streamWorktrees.map(async ({ wt, stream: matchedStream }) => {
          if (!wt.branch) return { worktree: wt, stream: matchedStream, ahead: 0, behind: 0 };
          const counts = await getAheadBehind(streamId, WORKTREE_BASE, wt.branch);
          return { worktree: wt, stream: matchedStream, ahead: counts.ahead, behind: counts.behind };
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

  useEffect(() => {
    if (!streamId) return;
    const unsubGit = subscribeGitRefsEvents(streamId, () => void refresh());
    const unsubWorkspace = subscribeWorkspaceEvents(streamId, () => void refresh());
    return () => {
      unsubGit();
      unsubWorkspace();
    };
  }, [streamId, refresh]);

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
    async (label: string, command: string, action: () => Promise<GitOpResult>) => {
      const ok = window.confirm(`${label}\n\nWill run:\n  ${command}\n\nProceed?`);
      if (!ok) return;
      setPendingAction(label);
      try {
        const result = await action();
        if (!result.ok) {
          window.alert(`${label} failed:\n${result.stderr || "git error"}`);
        } else {
          await refresh();
        }
      } finally {
        setPendingAction(null);
      }
    },
    [refresh],
  );

  if (!streamId) {
    return (
      <Page testId="page-git-dashboard" title="Git dashboard" kind="git">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-git-dashboard" title="Git dashboard" kind="git">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}

        {data ? (
          <>
            <BranchHeaderCard
              data={data.branchHeader}
              onPushUpstream={() =>
                runConfirmed(
                  "Push to upstream",
                  `git push${data.branchHeader.branch ? ` origin ${data.branchHeader.branch}` : ""}`,
                  () => gitPush(streamId),
                )
              }
              pending={pendingAction === "Push to upstream"}
            />

            <UncommittedMiniCard
              summary={data.uncommitted}
              onView={() => onOpenPage(uncommittedChangesRef())}
            />

            <RecentCommitsCard
              log={data.recentLog}
              onSelectCommit={handleSelectCommit}
              onViewFullHistory={() => onRevealCommit(data.recentLog.commits[0]?.sha ?? "")}
            />

            <StreamsCard
              streamId={streamId}
              rows={data.streams}
              workingByStreamId={streamWorkingFlags}
              onMerge={(branch) =>
                runConfirmed(
                  `Merge ${branch} into current`,
                  `git merge ${branch}`,
                  () => gitMergeInto(streamId, branch),
                )
              }
              pendingAction={pendingAction}
            />

            <RemoteBranchesCard
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
              pendingAction={pendingAction}
            />
          </>
        ) : null}
      </div>
    </Page>
  );
}

function BranchHeaderCard({
  data,
  onPushUpstream,
  pending,
}: {
  data: DashboardData["branchHeader"];
  onPushUpstream(): void;
  pending: boolean;
}) {
  return (
    <Card testId="git-dashboard-branch-header" title="Branch">
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <div style={{ fontSize: 18, fontWeight: 600 }}>{data.branch ?? "(detached)"}</div>
        {data.upstream ? (
          <div style={subtle}>
            tracks <code>{data.upstream}</code> · ahead {data.aheadUpstream}, behind{" "}
            {data.behindUpstream}
          </div>
        ) : (
          <div style={subtle}>no upstream tracking branch</div>
        )}
        {data.headSha ? (
          <div style={subtle}>
            HEAD <code>{data.headSha.slice(0, 7)}</code> {data.headSubject}
            {data.headDate ? ` · ${formatDate(data.headDate)}` : ""}
          </div>
        ) : null}
        <div style={{ marginTop: 6 }}>
          <button
            type="button"
            data-testid="git-dashboard-push-upstream"
            onClick={onPushUpstream}
            disabled={pending}
            style={primaryButton}
          >
            {pending ? "Pushing…" : "Push to upstream"}
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
    <Card testId="git-dashboard-uncommitted-mini" title="Uncommitted">
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>{total === 0 ? "No uncommitted files" : `${total} uncommitted file${total === 1 ? "" : "s"}`}</div>
        <button
          type="button"
          data-testid="git-dashboard-view-uncommitted"
          onClick={onView}
          style={linkButton}
        >
          View uncommitted →
        </button>
      </div>
    </Card>
  );
}

function RecentCommitsCard({
  log,
  onSelectCommit,
  onViewFullHistory,
}: {
  log: GitLogResult;
  onSelectCommit(sha: string): void;
  onViewFullHistory(): void;
}) {
  const refIndex = useMemo(() => indexRefsBySha(log), [log]);
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
          onSelect={onSelectCommit}
        />
      )}
    </Card>
  );
}

function StreamsCard({
  streamId,
  rows,
  onMerge,
  pendingAction,
  workingByStreamId,
}: {
  streamId: string;
  rows: StreamWorktreeRow[];
  onMerge(branch: string): void;
  pendingAction: string | null;
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
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 6, fontWeight: 500 }}>
                      {workingByStreamId[row.stream.id] ? (
                        <AgentStatusDot status="working" />
                      ) : null}
                      <span>{row.stream.title}</span>
                    </div>
                    <div style={{ ...subtle, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {branch}
                    </div>
                  </div>
                  <div style={subtle}>
                    ↑{row.ahead} ↓{row.behind}
                  </div>
                  {row.worktree.branch ? (
                    <button
                      type="button"
                      onClick={() => onMerge(row.worktree.branch!)}
                      disabled={pendingAction === mergeLabel}
                      style={smallButton}
                    >
                      {pendingAction === mergeLabel ? "Merging…" : "Merge into current"}
                    </button>
                  ) : null}
                </div>
                {isOpen && row.worktree.branch ? (
                  <PairwiseDiffPane
                    streamId={streamId}
                    rows={rows}
                    selfBranch={row.worktree.branch}
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

function PairwiseDiffPane({
  streamId,
  rows,
  selfBranch,
}: {
  streamId: string;
  rows: StreamWorktreeRow[];
  selfBranch: string;
}) {
  const otherBranches = rows
    .map((r) => r.worktree.branch)
    .filter((b): b is string => !!b && b !== selfBranch);
  const [target, setTarget] = useState<string>(otherBranches[0] ?? "");
  const [commits, setCommits] = useState<GitLogCommit[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!target) {
      setCommits([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void getCommitsAheadOf(streamId, target, selfBranch, 20)
      .then((result) => {
        if (!cancelled) setCommits(result);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [streamId, selfBranch, target]);

  if (otherBranches.length === 0) {
    return <div style={{ ...subtle, padding: "4px 0 8px 26px" }}>No other worktrees to compare with.</div>;
  }
  return (
    <div
      data-testid="git-dashboard-worktree-pairwise"
      style={{ padding: "4px 0 8px 26px", display: "flex", flexDirection: "column", gap: 6 }}
    >
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span style={subtle}>Commits in <code>{selfBranch}</code> not in</span>
        <select
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          style={{ fontSize: 12 }}
        >
          {otherBranches.map((b) => (
            <option key={b} value={b}>{b}</option>
          ))}
        </select>
      </div>
      {loading ? (
        <div style={subtle}>Loading…</div>
      ) : commits.length === 0 ? (
        <div style={subtle}>No commits ahead.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {commits.map((c) => (
            <div key={c.sha} style={{ display: "flex", gap: 8, fontSize: 12 }}>
              <code style={{ color: "var(--text-muted)" }}>{c.sha.slice(0, 7)}</code>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {c.commit.message}
              </span>
              <span style={subtle}>{c.commit.author.name}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function RemoteBranchesCard({
  rows,
  onPull,
  onPush,
  pendingAction,
}: {
  rows: RemoteBranchEntry[];
  onPull(remote: string, branch: string): void;
  onPush(remote: string, branch: string): void;
  pendingAction: string | null;
}) {
  return (
    <Card testId="git-dashboard-remote-branches" title="Recent remote branches">
      {rows.length === 0 ? (
        <div style={muted}>No remote branches.</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row) => {
            const pullLabel = `Pull ${row.shortName} into current`;
            const pushLabel = `Push current → ${row.shortName}`;
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
                <button
                  type="button"
                  onClick={() => onPull(row.remote, row.branch)}
                  disabled={pendingAction === pullLabel}
                  style={smallButton}
                >
                  {pendingAction === pullLabel ? "Pulling…" : "Pull into current"}
                </button>
                <button
                  type="button"
                  onClick={() => onPush(row.remote, row.branch)}
                  disabled={pendingAction === pushLabel}
                  style={smallButton}
                >
                  {pendingAction === pushLabel ? "Pushing…" : "Push current →"}
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

