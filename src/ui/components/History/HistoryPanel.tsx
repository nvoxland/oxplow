import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { GitLogCommit, GitLogResult, Stream } from "../../api.js";
import { getGitLog, subscribeGitRefsEvents } from "../../api.js";
import { logUi } from "../../logger.js";
import { CommitGraphTable, indexRefsBySha } from "./CommitGraphTable.js";

interface Props {
  stream: Stream | null;
  /**
   * Called when a commit row is clicked. The host wires this to navigate
   * to the per-commit page so commits are bookmark/back/forward citizens
   * (no longer an inline detail pane on this panel).
   */
  onSelectCommit?(sha: string, opts?: { newTab?: boolean }): void;
  /** Optional sha to scroll into view (e.g. when arriving from blame). */
  revealSha?: { sha: string; token: number } | null;
}

export function HistoryPanel({ stream, onSelectCommit, revealSha }: Props) {
  const [log, setLog] = useState<GitLogResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [author, setAuthor] = useState("");
  const [branch, setBranch] = useState("");
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
    if (!revealSha) return;
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

  const refIndex = useMemo(() => indexRefsBySha(log), [log]);

  const matchCount = matches ? matches.size : visibleCommits.length;

  return (
    <div id="history-panel-root" style={containerStyle}>
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
        ) : !log ? null : (
          <CommitGraphTable
            commits={visibleCommits}
            branchHeadsBySha={refIndex.branchHeadsBySha}
            tagsBySha={refIndex.tagsBySha}
            currentBranch={log.currentBranch}
            selectedSha={revealSha?.sha ?? null}
            matches={matches}
            onSelect={(sha, opts) => onSelectCommit?.(sha, opts)}
            rowRefs={rowRefs}
          />
        )}
      </div>
    </div>
  );
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

const containerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  overflow: "hidden",
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
