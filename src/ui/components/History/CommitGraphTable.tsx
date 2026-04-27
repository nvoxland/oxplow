import type { CSSProperties, MutableRefObject } from "react";
import { useMemo } from "react";
import type { GitLogCommit, GitLogResult } from "../../api.js";
import { layoutCommits, type GraphRow } from "./layout.js";

const BRANCH_COLORS = [
  "#4a9eff",
  "#86efac",
  "#e5a06a",
  "#c4b5fd",
  "#f472b6",
  "#fcd34d",
  "#60d394",
  "#f87171",
];

const ROW_HEIGHT = 36;
const LANE_WIDTH = 14;
const NODE_RADIUS = 4;
const GRAPH_PAD = 8;

export interface CommitGraphTableProps {
  commits: GitLogCommit[];
  branchHeadsBySha: Map<string, string[]>;
  tagsBySha: Map<string, string[]>;
  currentBranch: string | null;
  selectedSha?: string | null;
  /** When set, rows whose sha is NOT in this set render at 35% opacity. Pass `null` to disable highlighting (every row treated as matched). */
  matches?: Set<string> | null;
  onSelect?(sha: string, opts?: { newTab?: boolean }): void;
  /** Optional row-element ref map for scroll-into-view. */
  rowRefs?: MutableRefObject<Map<string, HTMLDivElement>>;
}

/**
 * The git-log graph + row table — branch/merge dots and lines on the
 * left, sha + ref badges + subject + author + relative date on the
 * right. Pure presentation against a pre-filtered commit list. Both
 * `HistoryPanel` and `GitDashboardPage` use this; the dashboard
 * passes `commits.slice(0, 5)` and no matches set.
 */
export function CommitGraphTable({
  commits,
  branchHeadsBySha,
  tagsBySha,
  currentBranch,
  selectedSha,
  matches,
  onSelect,
  rowRefs,
}: CommitGraphTableProps) {
  const layout = useMemo(() => layoutCommits(commits), [commits]);
  const graphWidth = Math.max(
    GRAPH_PAD * 2 + LANE_WIDTH * Math.max(1, layout.totalColumns),
    GRAPH_PAD * 2 + LANE_WIDTH,
  );

  if (layout.rows.length === 0) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No commits.</div>;
  }

  return (
    <div data-testid="commit-graph-table">
      {layout.rows.map((row) => {
        const sha = row.commit.sha;
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
            <CommitRow
              row={row}
              graphWidth={graphWidth}
              selected={selectedSha === sha}
              matched={matched}
              branchHeads={branchHeadsBySha.get(sha) ?? []}
              tags={tagsBySha.get(sha) ?? []}
              currentBranch={currentBranch}
              onClick={(e) => onSelect?.(sha, { newTab: e.metaKey || e.ctrlKey || e.button === 1 })}
            />
          </div>
        );
      })}
    </div>
  );
}

/**
 * Group log refs (branch heads / tags) by sha so the table can
 * overlay them next to each row. Same shape used by both surfaces.
 */
export function indexRefsBySha(log: GitLogResult | null): {
  branchHeadsBySha: Map<string, string[]>;
  tagsBySha: Map<string, string[]>;
} {
  const branchHeadsBySha = new Map<string, string[]>();
  const tagsBySha = new Map<string, string[]>();
  if (!log) return { branchHeadsBySha, tagsBySha };
  for (const head of log.branchHeads) {
    const list = branchHeadsBySha.get(head.commit.sha) ?? [];
    list.push(head.name);
    branchHeadsBySha.set(head.commit.sha, list);
  }
  for (const tag of log.tags) {
    const list = tagsBySha.get(tag.commit.sha) ?? [];
    list.push(tag.name);
    tagsBySha.set(tag.commit.sha, list);
  }
  return { branchHeadsBySha, tagsBySha };
}

function CommitRow({
  row,
  graphWidth,
  selected,
  matched,
  branchHeads,
  tags,
  currentBranch,
  onClick,
}: {
  row: GraphRow;
  graphWidth: number;
  selected: boolean;
  matched: boolean;
  branchHeads: string[];
  tags: string[];
  currentBranch: string | null;
  onClick(e: { metaKey: boolean; ctrlKey: boolean; button: number }): void;
}) {
  const mid = ROW_HEIGHT / 2;
  const colX = (k: number) => GRAPH_PAD + k * LANE_WIDTH + LANE_WIDTH / 2;

  const lines: Array<{ x1: number; y1: number; x2: number; y2: number; color: string; key: string }> = [];

  for (let k = 0; k < row.incoming.length; k++) {
    const sha = row.incoming[k];
    if (!sha) continue;
    const color = BRANCH_COLORS[k % BRANCH_COLORS.length]!;
    if (k === row.column) {
      if (row.fromAbove) lines.push({ x1: colX(k), y1: 0, x2: colX(k), y2: mid, color, key: `in-${k}` });
    } else {
      lines.push({ x1: colX(k), y1: 0, x2: colX(k), y2: ROW_HEIGHT, color, key: `in-${k}` });
    }
  }

  for (let k = 0; k < row.outgoing.length; k++) {
    const sha = row.outgoing[k];
    if (!sha) continue;
    const incomingSame = row.incoming[k] === sha;
    if (incomingSame && k !== row.column) continue;
    const color = BRANCH_COLORS[k % BRANCH_COLORS.length]!;
    if (k === row.column) {
      lines.push({ x1: colX(k), y1: mid, x2: colX(k), y2: ROW_HEIGHT, color, key: `out-${k}` });
    }
  }

  for (const edge of row.parentEdges) {
    const color = BRANCH_COLORS[edge.toCol % BRANCH_COLORS.length]!;
    if (edge.toCol === row.column) continue;
    lines.push({
      x1: colX(row.column),
      y1: mid,
      x2: colX(edge.toCol),
      y2: ROW_HEIGHT,
      color,
      key: `edge-${edge.toCol}-${edge.sha}`,
    });
  }

  const nodeColor = BRANCH_COLORS[row.column % BRANCH_COLORS.length]!;
  const date = formatTimestamp(row.commit.commit.author.date);

  return (
    <div
      onClick={onClick}
      data-testid="commit-graph-row"
      data-sha={row.commit.sha}
      style={{
        display: "flex",
        alignItems: "center",
        height: ROW_HEIGHT,
        cursor: "pointer",
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        opacity: matched ? 1 : 0.35,
        fontSize: 13,
        lineHeight: 1,
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      <svg width={graphWidth} height={ROW_HEIGHT} style={{ flexShrink: 0 }}>
        {lines.map((line) => (
          <line
            key={line.key}
            x1={line.x1}
            y1={line.y1}
            x2={line.x2}
            y2={line.y2}
            stroke={line.color}
            strokeWidth={1.5}
            strokeLinecap="round"
          />
        ))}
        <circle
          cx={colX(row.column)}
          cy={mid}
          r={NODE_RADIUS}
          fill={selected ? "#fff" : nodeColor}
          stroke={nodeColor}
          strokeWidth={1.5}
        />
      </svg>
      <div style={{ display: "flex", alignItems: "center", gap: 6, flex: 1, minWidth: 0, paddingRight: 8 }}>
        <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--muted)", flexShrink: 0, fontSize: 11 }}>
          {row.commit.sha.slice(0, 7)}
        </span>
        {branchHeads.map((name) => (
          <RefBadge key={`b-${name}`} label={name} tone={name === currentBranch ? "current" : "branch"} />
        ))}
        {tags.map((name) => (
          <RefBadge key={`t-${name}`} label={name} tone="tag" />
        ))}
        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
          {row.commit.commit.message}
        </span>
        <span style={{ color: "var(--muted)", flexShrink: 0, fontSize: 11 }}>
          {row.commit.commit.author.name}
        </span>
        <span
          style={{ color: "var(--muted)", flexShrink: 0, fontSize: 11, minWidth: 132, textAlign: "right" }}
          title={row.commit.commit.author.date}
        >
          {date}
        </span>
      </div>
    </div>
  );
}

function RefBadge({ label, tone }: { label: string; tone: "branch" | "current" | "tag" }) {
  const styles: Record<typeof tone, CSSProperties> = {
    branch: { borderColor: "#4a9eff", color: "#4a9eff" },
    current: { borderColor: "#86efac", color: "#86efac", fontWeight: 600 },
    tag: { borderColor: "#fcd34d", color: "#fcd34d" },
  };
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        border: "1px solid",
        borderRadius: 999,
        padding: "0 6px",
        fontSize: 10,
        lineHeight: "14px",
        flexShrink: 0,
        ...styles[tone],
      }}
      title={tone === "tag" ? `tag: ${label}` : label}
    >
      {tone === "tag" ? "🏷 " : ""}{label}
    </span>
  );
}

/**
 * Local-time timestamp like `2026-04-26 14:32` — readable at a glance
 * and stable as the row scrolls past. The "time since" framing is
 * computed on the dedicated commit/history page the row links to,
 * not in this row.
 */
export function formatTimestamp(input: string): string {
  if (!input) return "";
  const date = new Date(input);
  if (Number.isNaN(date.getTime())) return input;
  const yyyy = date.getFullYear();
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  const hh = String(date.getHours()).padStart(2, "0");
  const min = String(date.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${min}`;
}
