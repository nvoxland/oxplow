import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CommitDetail, GitLogCommit, GitLogResult, Stream } from "../../api.js";
import { getCommitDetail, getGitLog, subscribeGitRefsEvents } from "../../api.js";
import { logUi } from "../../logger.js";
import { layoutCommits, type GraphRow } from "./layout.js";
import type { DiffRequest } from "../Diff/diff-request.js";

interface Props {
  stream: Stream | null;
  onOpenDiff?(request: DiffRequest): void;
  revealSha?: { sha: string; token: number } | null;
}

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

const ROW_HEIGHT = 24;
const LANE_WIDTH = 14;
const NODE_RADIUS = 4;
const GRAPH_PAD = 8;

export function HistoryPanel({ stream, onOpenDiff, revealSha }: Props) {
  const [log, setLog] = useState<GitLogResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [author, setAuthor] = useState("");
  const [branch, setBranch] = useState("");
  const [selectedSha, setSelectedSha] = useState<string | null>(null);
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailWidth, setDetailWidth] = useState<number>(380);
  const [dragging, setDragging] = useState(false);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());
  const limit = 500;

  useEffect(() => {
    if (!stream) {
      setLog(null);
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const load = (opts?: { silent?: boolean }) => {
      if (!opts?.silent) setLoading(true);
      setError(null);
      void getGitLog(stream.id, { limit })
        .then((result) => {
          if (cancelled) return;
          setLog(result);
          setLoading(false);
        })
        .catch((err) => {
          if (cancelled) return;
          logUi("warn", "git log failed", { error: String(err) });
          setError(String(err));
          setLoading(false);
        });
    };
    load();
    const unsubscribe = subscribeGitRefsEvents(stream.id, () => {
      if (timer) clearTimeout(timer);
      // Refresh silently on external git events so the list doesn't flash a
      // loading spinner every time the agent commits.
      timer = setTimeout(() => load({ silent: true }), 150);
    });
    return () => {
      cancelled = true;
      unsubscribe();
      if (timer) clearTimeout(timer);
    };
  }, [stream?.id]);

  useEffect(() => {
    if (!stream || !selectedSha) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    void getCommitDetail(stream.id, selectedSha)
      .then((result) => {
        if (cancelled) return;
        setDetail(result);
        setDetailLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "commit detail failed", { error: String(err) });
        setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream?.id, selectedSha]);

  useEffect(() => {
    if (!dragging) return;
    const onMove = (event: PointerEvent) => {
      const container = document.getElementById("history-panel-root");
      if (!container) return;
      const rect = container.getBoundingClientRect();
      const next = rect.right - event.clientX;
      setDetailWidth(Math.min(Math.max(next, 240), Math.max(rect.width - 300, 240)));
    };
    const onUp = () => setDragging(false);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    const prevCursor = document.body.style.cursor;
    document.body.style.cursor = "col-resize";
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      document.body.style.cursor = prevCursor;
    };
  }, [dragging]);

  useEffect(() => {
    if (!revealSha) return;
    setSelectedSha(revealSha.sha);
    // Defer to let rows render before scrolling.
    requestAnimationFrame(() => {
      const node = rowRefs.current.get(revealSha.sha);
      if (node) node.scrollIntoView({ block: "nearest" });
    });
  }, [revealSha?.token, revealSha?.sha]);

  const authors = useMemo(() => {
    if (!log) return [] as string[];
    const set = new Set<string>();
    for (const commit of log.commits) set.add(commit.commit.author.name);
    return [...set].sort((a, b) => a.localeCompare(b));
  }, [log]);

  const reachableShas = useMemo(() => reachableFromBranch(log, branch), [log, branch]);

  const visibleCommits = useMemo(() => {
    if (!log) return [] as GitLogCommit[];
    if (!reachableShas) return log.commits;
    return log.commits.filter((c) => reachableShas.has(c.sha));
  }, [log, reachableShas]);

  const queryLower = query.trim().toLowerCase();
  const matches = useMemo(() => {
    if (!queryLower && !author) return null;
    const out = new Set<string>();
    for (const commit of visibleCommits) {
      if (author && commit.commit.author.name !== author) continue;
      if (!queryLower) { out.add(commit.sha); continue; }
      const hit = commit.sha.toLowerCase().includes(queryLower)
        || commit.commit.message.toLowerCase().includes(queryLower)
        || commit.commit.author.name.toLowerCase().includes(queryLower)
        || commit.commit.author.email.toLowerCase().includes(queryLower);
      if (hit) out.add(commit.sha);
    }
    return out;
  }, [visibleCommits, queryLower, author]);

  const layout = useMemo(() => layoutCommits(visibleCommits), [visibleCommits]);

  const branchHeadBySha = useMemo(() => {
    const map = new Map<string, string[]>();
    if (!log) return map;
    for (const head of log.branchHeads) {
      const list = map.get(head.commit.sha) ?? [];
      list.push(head.name);
      map.set(head.commit.sha, list);
    }
    return map;
  }, [log]);
  const tagBySha = useMemo(() => {
    const map = new Map<string, string[]>();
    if (!log) return map;
    for (const tag of log.tags) {
      const list = map.get(tag.commit.sha) ?? [];
      list.push(tag.name);
      map.set(tag.commit.sha, list);
    }
    return map;
  }, [log]);

  const graphWidth = Math.max(
    GRAPH_PAD * 2 + LANE_WIDTH * Math.max(1, layout.totalColumns),
    GRAPH_PAD * 2 + LANE_WIDTH,
  );

  const matchCount = matches ? matches.size : visibleCommits.length;

  return (
    <div id="history-panel-root" style={containerStyle}>
      <div style={{ display: "flex", flex: 1, minHeight: 0, minWidth: 0 }}>
        <div style={leftPaneStyle}>
          <div style={toolbarStyle}>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter commits (message, sha, author)"
              style={{ ...inputStyle, flex: 1, minWidth: 160 }}
            />
            <select value={author} onChange={(e) => setAuthor(e.target.value)} style={inputStyle}>
              <option value="">All authors</option>
              {authors.map((name) => (<option key={name} value={name}>{name}</option>))}
            </select>
            <select value={branch} onChange={(e) => setBranch(e.target.value)} style={inputStyle}>
              <option value="">All branches</option>
              {(log?.branchHeads ?? []).map((b) => (
                <option key={b.name} value={b.name}>{b.name}</option>
              ))}
            </select>
            <div style={{ color: "var(--muted)", fontSize: 11, marginLeft: "auto", whiteSpace: "nowrap" }}>
              {loading ? "loading…" : log ? `${matchCount} / ${log.commits.length}` : ""}
            </div>
          </div>
          <div style={listStyle}>
            {error ? (
              <div style={{ padding: 12, color: "#ff6b6b", fontSize: 12 }}>{error}</div>
            ) : !stream ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No stream selected.</div>
            ) : !log ? null : layout.rows.length === 0 ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No commits.</div>
            ) : (
              layout.rows.map((row, index) => {
                const next = layout.rows[index + 1] ?? null;
                const matched = !matches || matches.has(row.commit.sha);
                const sha = row.commit.sha;
                return (
                  <div
                    key={sha}
                    ref={(node) => {
                      if (node) rowRefs.current.set(sha, node);
                      else rowRefs.current.delete(sha);
                    }}
                  >
                    <CommitRow
                      row={row}
                      nextRow={next}
                      graphWidth={graphWidth}
                      selected={selectedSha === sha}
                      matched={matched}
                      branchHeads={branchHeadBySha.get(sha) ?? []}
                      tags={tagBySha.get(sha) ?? []}
                      currentBranch={log.currentBranch}
                      onClick={() => setSelectedSha(sha)}
                    />
                  </div>
                );
              })
            )}
          </div>
        </div>
        <div
          onPointerDown={(e) => { e.preventDefault(); setDragging(true); }}
          style={{ ...resizeHandleStyle, background: dragging ? "var(--accent)" : undefined }}
        />
        <div style={{ ...detailPaneStyle, width: detailWidth }}>
          <DetailPane
            detail={detail}
            loading={detailLoading}
            sha={selectedSha}
            onOpenFileDiff={(sha, path, parent) => {
              if (!onOpenDiff) return;
              // Double-clicking a file in the commit's file list should show
              // the change that commit introduced. Diff its parent against
              // the commit — for root commits use the empty tree hash.
              const left = parent ?? "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
              onOpenDiff({
                path,
                leftRef: left,
                rightKind: { ref: sha },
                baseLabel: parent ? parent.slice(0, 7) : "(root)",
              });
            }}
          />
        </div>
      </div>
    </div>
  );
}

function CommitRow({
  row,
  nextRow,
  graphWidth,
  selected,
  matched,
  branchHeads,
  tags,
  currentBranch,
  onClick,
}: {
  row: GraphRow;
  nextRow: GraphRow | null;
  graphWidth: number;
  selected: boolean;
  matched: boolean;
  branchHeads: string[];
  tags: string[];
  currentBranch: string | null;
  onClick(): void;
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
      // Pass-through lane: full vertical top to bottom.
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
    if (edge.toCol === row.column) continue; // straight down handled by outgoing
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
  const date = formatRelative(row.commit.commit.author.date);

  return (
    <div
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        height: ROW_HEIGHT,
        cursor: "pointer",
        background: selected ? "rgba(74, 158, 255, 0.18)" : "transparent",
        opacity: matched ? 1 : 0.35,
        fontSize: 12,
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
        <span style={{ color: "var(--muted)", flexShrink: 0, fontSize: 11, minWidth: 64, textAlign: "right" }}>
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

function DetailPane({
  detail,
  loading,
  sha,
  onOpenFileDiff,
}: {
  detail: CommitDetail | null;
  loading: boolean;
  sha: string | null;
  onOpenFileDiff?(sha: string, path: string, parent: string | null): void;
}) {
  if (!sha) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Select a commit to see details.</div>;
  }
  if (loading && !detail) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Loading…</div>;
  }
  if (!detail) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Commit not found.</div>;
  }
  const totalAdditions = detail.files.reduce((sum, f) => sum + f.additions, 0);
  const totalDeletions = detail.files.reduce((sum, f) => sum + f.deletions, 0);
  return (
    <div style={{ padding: 10, display: "flex", flexDirection: "column", gap: 10, fontSize: 12, overflow: "auto", height: "100%" }}>
      <div>
        <div style={{ fontWeight: 600, marginBottom: 4 }}>{detail.subject}</div>
        {detail.body ? (
          <div style={{ whiteSpace: "pre-wrap", color: "var(--muted)", fontSize: 11 }}>{detail.body}</div>
        ) : null}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 10px", color: "var(--muted)", fontSize: 11 }}>
        <span>SHA</span><span style={{ fontFamily: "var(--mono, monospace)", color: "inherit" }}>{detail.sha}</span>
        <span>Author</span><span>{detail.author.name}{detail.author.email ? ` <${detail.author.email}>` : ""}</span>
        <span>Date</span><span>{formatAbsolute(detail.author.date)}</span>
        {detail.committer.email && detail.committer.email !== detail.author.email ? (
          <>
            <span>Committer</span><span>{detail.committer.name}{detail.committer.email ? ` <${detail.committer.email}>` : ""}</span>
            <span>Committed</span><span>{formatAbsolute(detail.committer.date)}</span>
          </>
        ) : null}
        {detail.parents.length > 0 ? (
          <>
            <span>Parents</span>
            <span style={{ fontFamily: "var(--mono, monospace)" }}>
              {detail.parents.map((p) => p.slice(0, 7)).join(", ")}
            </span>
          </>
        ) : null}
      </div>
      <div>
        <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)", marginBottom: 4 }}>
          {detail.files.length} file{detail.files.length === 1 ? "" : "s"} changed
          <span style={{ marginLeft: 6, color: "#86efac" }}>+{totalAdditions}</span>
          <span style={{ marginLeft: 4, color: "#f87171" }}>−{totalDeletions}</span>
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {detail.files.map((file) => (
            <div
              key={file.path}
              onDoubleClick={() => {
                if (!onOpenFileDiff || !detail) return;
                // Strip rename decoration ("from → to") to keep just the current path.
                const realPath = file.path.includes(" → ") ? file.path.split(" → ")[1]! : file.path;
                onOpenFileDiff(detail.sha, realPath, detail.parents[0] ?? null);
              }}
              title="Double-click to open diff"
              style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, cursor: onOpenFileDiff ? "pointer" : "default" }}
            >
              <span style={{ ...statusBadgeStyle, color: statusColor(file.status) }}>{statusLabel(file.status)}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }} title={file.path}>{file.path}</span>
              {file.additions > 0 ? <span style={{ color: "#86efac" }}>+{file.additions}</span> : null}
              {file.deletions > 0 ? <span style={{ color: "#f87171" }}>−{file.deletions}</span> : null}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case "added": return "A";
    case "deleted": return "D";
    case "modified": return "M";
    case "renamed": return "R";
    case "untracked": return "?";
    default: return "·";
  }
}

function statusColor(status: string): string {
  switch (status) {
    case "added": return "#86efac";
    case "deleted": return "#f87171";
    case "modified": return "#e5a06a";
    case "renamed": return "#c4b5fd";
    default: return "var(--muted)";
  }
}

function reachableFromBranch(log: GitLogResult | null, branch: string): Set<string> | null {
  if (!log || !branch) return null;
  const head = log.branchHeads.find((b) => b.name === branch);
  if (!head) return new Set();
  const parentsBySha = new Map<string, string[]>();
  for (const commit of log.commits) {
    parentsBySha.set(commit.sha, commit.parents.map((p) => p.sha));
  }
  const reachable = new Set<string>();
  const stack = [head.commit.sha];
  while (stack.length > 0) {
    const sha = stack.pop()!;
    if (reachable.has(sha)) continue;
    reachable.add(sha);
    for (const parent of parentsBySha.get(sha) ?? []) stack.push(parent);
  }
  return reachable;
}

function formatRelative(input: string): string {
  if (!input) return "";
  const then = new Date(input).getTime();
  if (!Number.isFinite(then)) return input;
  const diff = Date.now() - then;
  const sec = Math.round(diff / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.round(hr / 24);
  if (day < 30) return `${day}d`;
  const mon = Math.round(day / 30);
  if (mon < 12) return `${mon}mo`;
  return `${Math.round(mon / 12)}y`;
}

function formatAbsolute(input: string): string {
  if (!input) return "";
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleString();
}

const containerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  overflow: "hidden",
};

const leftPaneStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  flex: 1,
  minWidth: 0,
  minHeight: 0,
};

const toolbarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: 6,
  borderBottom: "1px solid var(--border)",
  flexWrap: "wrap",
};

const inputStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  font: "inherit",
  padding: "3px 6px",
  fontSize: 12,
};

const listStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflow: "auto",
  background: "var(--bg)",
};

const resizeHandleStyle: CSSProperties = {
  width: 4,
  flexShrink: 0,
  cursor: "col-resize",
  borderLeft: "1px solid var(--border)",
};

const detailPaneStyle: CSSProperties = {
  flexShrink: 0,
  minWidth: 240,
  maxWidth: "100%",
  borderLeft: "1px solid var(--border)",
  background: "var(--bg-2)",
  minHeight: 0,
  display: "flex",
  flexDirection: "column",
  overflow: "hidden",
};

const statusBadgeStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 16,
  fontFamily: "var(--mono, monospace)",
  fontSize: 10,
  fontWeight: 600,
  flexShrink: 0,
};
