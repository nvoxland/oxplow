import type { MutableRefObject } from "react";
import { useMemo } from "react";
import type { RevisionInfo } from "../../tauri-bridge/generated/bindings.js";
import type { History } from "../../vcsHistory.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { gitCommitRef } from "../../tabs/pageRefs.js";
import type { OriginItem } from "../../tabs/session.js";
import { RefBadge } from "../RefBadge.js";
import { EmptyState } from "../Prompts/EmptyState.js";

const ROW_HEIGHT = 36;

export interface CommitStats {
  filesAdded: number;
  filesModified: number;
  filesDeleted: number;
  additions: number;
  deletions: number;
}

export interface CommitListProps {
  commits: RevisionInfo[];
  branchHeadsBySha: Map<string, string[]>;
  tagsBySha: Map<string, string[]>;
  currentBranch: string | null;
  selectedSha?: string | null;
  /** When set, rows whose sha is NOT in this set render at 35% opacity. Pass `null` to disable highlighting (every row treated as matched). */
  matches?: Set<string> | null;
  /** Optional per-commit stats. When present, an extra `A# M# D# +# −#` cell renders before the author column. */
  statsBySha?: Map<string, CommitStats>;
  onSelect?(sha: string, opts?: { newTab?: boolean }): void;
  /** Optional row-element ref map for scroll-into-view. */
  rowRefs?: MutableRefObject<Map<string, HTMLDivElement>>;
}

/**
 * Commits as a plain list — sha + ref badges + subject + author + date,
 * newest first. Provider-neutral: no lanes (oxplow-bundled's
 * `history-graph` component draws those, replacing this on Git History —
 * the `vcs.history.graph` replacement). Pure presentation against a
 * pre-filtered commit list; `GitDashboardPage` passes its recent commits
 * and their stats.
 */
export function CommitList({
  commits,
  branchHeadsBySha,
  tagsBySha,
  currentBranch,
  selectedSha,
  matches,
  statsBySha,
  onSelect,
  rowRefs,
}: CommitListProps) {
  // The origin for Up/Down: the destination GitCommitPage steps through
  // the same list the user clicked from.
  const siblingEntries: OriginItem[] = useMemo(
    () =>
      commits.map((commit) => ({
        ref: gitCommitRef(commit.id),
        label: `${commit.id.slice(0, 7)} ${commit.subject ?? ""}`.trim(),
      })),
    [commits],
  );

  if (commits.length === 0) {
    return (
      <div style={{ padding: 12 }}>
        <EmptyState testId="history-empty" title="No commits" text="This branch has no history yet." />
      </div>
    );
  }

  return (
    <div data-testid="commit-list">
      {commits.map((commit, idx) => {
        const sha = commit.id;
        const matched = !matches || matches.has(sha);
        return (
          <div
            key={sha}
            ref={(node) => {
              if (!rowRefs) return;
              if (node) rowRefs.current.set(sha, node);
              else rowRefs.current.delete(sha);
            }}
          >
            <CommitRowDispatcher
              sha={sha}
              commit={commit}
              selected={selectedSha === sha}
              matched={matched}
              branchHeads={branchHeadsBySha.get(sha) ?? []}
              tags={tagsBySha.get(sha) ?? []}
              currentBranch={currentBranch}
              stats={statsBySha?.get(sha) ?? null}
              origin={{ items: siblingEntries, index: idx, title: "Recent commits" }}
              onSelect={onSelect}
            />
          </div>
        );
      })}
    </div>
  );
}

/**
 * Adapter that owns the per-row `useRouteDispatch` so the page-context
 * path picks up siblings while the `onSelect` callback (rail, dashboard)
 * is the new-tab fallback.
 */
function CommitRowDispatcher({
  sha,
  commit,
  selected,
  matched,
  branchHeads,
  tags,
  currentBranch,
  stats,
  origin,
  onSelect,
}: {
  sha: string;
  commit: RevisionInfo;
  selected: boolean;
  matched: boolean;
  branchHeads: string[];
  tags: string[];
  currentBranch: string | null;
  stats: CommitStats | null;
  origin: { items: OriginItem[]; index: number; title?: string };
  onSelect?(sha: string, opts?: { newTab?: boolean }): void;
}) {
  const { dispatch } = useRouteDispatch(gitCommitRef(sha), {
    origin,
    onNavigate: (_ref, opts) => onSelect?.(sha, opts),
  });
  return (
    <CommitRow
      commit={commit}
      selected={selected}
      matched={matched}
      branchHeads={branchHeads}
      tags={tags}
      currentBranch={currentBranch}
      stats={stats}
      onClick={(e) => {
        const newTab = e.metaKey || e.ctrlKey || e.button === 1;
        dispatch(newTab);
      }}
    />
  );
}

/**
 * Group branch heads and tags by sha so the table can overlay them next
 * to each row. Same shape used by both surfaces.
 */
export function indexRefsBySha(history: Pick<History, "branchHeads" | "tags"> | null): {
  branchHeadsBySha: Map<string, string[]>;
  tagsBySha: Map<string, string[]>;
} {
  const branchHeadsBySha = new Map<string, string[]>();
  const tagsBySha = new Map<string, string[]>();
  if (!history) return { branchHeadsBySha, tagsBySha };
  for (const head of history.branchHeads) {
    branchHeadsBySha.set(head.sha, [...(branchHeadsBySha.get(head.sha) ?? []), head.name]);
  }
  for (const tag of history.tags) {
    tagsBySha.set(tag.sha, [...(tagsBySha.get(tag.sha) ?? []), tag.name]);
  }
  return { branchHeadsBySha, tagsBySha };
}

function CommitRow({
  commit,
  selected,
  matched,
  branchHeads,
  tags,
  currentBranch,
  stats,
  onClick,
}: {
  commit: RevisionInfo;
  selected: boolean;
  matched: boolean;
  branchHeads: string[];
  tags: string[];
  currentBranch: string | null;
  stats: CommitStats | null;
  onClick(e: { metaKey: boolean; ctrlKey: boolean; button: number }): void;
}) {
  const date = formatTimestamp(commit.time);

  return (
    <div
      onClick={onClick}
      data-testid="commit-list-row"
      data-sha={commit.id}
      data-ref-kind="commit"
      data-ref-id={commit.id}
      style={{
        display: "flex",
        alignItems: "center",
        height: ROW_HEIGHT,
        cursor: "pointer",
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        opacity: matched ? 1 : 0.35,
        fontSize: "var(--text-sm)",
        lineHeight: 1,
        whiteSpace: "nowrap",
        overflow: "hidden",
        paddingLeft: 8,
      }}
    >
      <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--muted)", fontSize: 11, flexShrink: 0, marginRight: 8 }}>
        {commit.id.slice(0, 7)}
      </span>
      <div style={{ display: "flex", alignItems: "center", gap: 6, flex: 1, minWidth: 0, paddingRight: 8 }}>
        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", userSelect: "text" }}>
          {commit.subject}
        </span>
        {branchHeads.map((name) => (
          <RefBadge key={`b-${name}`} label={name} tone={name === currentBranch ? "current" : "branch"} />
        ))}
        {tags.map((name) => (
          <RefBadge key={`t-${name}`} label={name} tone="tag" />
        ))}
        <span style={{ color: "var(--muted)", flexShrink: 0, fontSize: 11 }}>
          {commit.author}
        </span>
        {stats ? (
          <span
            data-testid="commit-list-row-stats"
            style={{
              flexShrink: 0,
              fontSize: 11,
              fontFamily: "var(--mono, monospace)",
              color: "var(--muted)",
              display: "inline-flex",
              gap: 6,
              whiteSpace: "pre",
            }}
            title={`${stats.filesAdded} added, ${stats.filesModified} modified, ${stats.filesDeleted} deleted · +${stats.additions} −${stats.deletions} lines`}
          >
            <span style={{ color: "var(--text-success, #16a34a)" }}>{`A${pad2(stats.filesAdded)}`}</span>
            <span>{`M${pad2(stats.filesModified)}`}</span>
            <span style={{ color: "var(--text-danger, #dc2626)" }}>{`D${pad2(stats.filesDeleted)}`}</span>
            <span style={{ color: "var(--text-success, #16a34a)" }}>{`+${pad3(stats.additions)}`}</span>
            <span style={{ color: "var(--text-danger, #dc2626)" }}>{`−${pad3(stats.deletions)}`}</span>
          </span>
        ) : null}
        <span
          style={{ color: "var(--muted)", flexShrink: 0, fontSize: 11, minWidth: 132, textAlign: "right" }}
          title={`${commit.time}\n${commit.id}`}
        >
          {date}
        </span>
      </div>
    </div>
  );
}

function pad2(n: number): string {
  return String(n).padStart(2, " ");
}

function pad3(n: number): string {
  return String(n).padStart(3, " ");
}

/**
 * Local-time timestamp like `2026-04-26 14:32` — readable at a glance
 * and stable as the row scrolls past. The "time since" framing is
 * computed on the dedicated commit/history page the row links to,
 * not in this row.
 */
export function formatTimestamp(input: string | number): string {
  if (!input && input !== 0) return "";
  // Bindings ship timestamps as Unix seconds; rows read through models
  // carry an ISO string. Accept both.
  const date =
    typeof input === "number"
      ? new Date(input * 1000)
      : new Date(input);
  if (Number.isNaN(date.getTime())) return String(input);
  const yyyy = date.getFullYear();
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  const hh = String(date.getHours()).padStart(2, "0");
  const min = String(date.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${min}`;
}
