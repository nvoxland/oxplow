import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Stream } from "../../api.js";
import { vcsHead } from "../../api.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { logUi } from "../../logger.js";
import { vcsRevOf } from "../../revision.js";
import type { RevisionInfo } from "../../tauri-bridge/generated/bindings.js";
import { readHistory, type History } from "../../vcsHistory.js";
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

/**
 * The stream's history: `v_commit` walked from the stream's head (through
 * the models — `.context/vcs.md`), re-read whenever a model it read
 * changes (the commit indexer runs on every ref move).
 */
export function HistoryPanel({ stream, onSelectCommit, revealSha }: Props) {
  const [log, setLog] = useState<(History & { currentBranch: string | null }) | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [author, setAuthor] = useState("");
  const [branch, setBranch] = useState("");
  const rowRefs = useRef(new Map<string, HTMLDivElement>());
  const limit = 500;
  const streamId = stream?.id ?? null;

  const load = useCallback(
    async (silent: boolean) => {
      if (!streamId) {
        setLog(null);
        return;
      }
      if (!silent) setLoading(true);
      setError(null);
      try {
        const head = await vcsHead(streamId);
        const history = await readHistory(vcsRevOf(head.revision), limit);
        setLog({ ...history, currentBranch: head.branch });
      } catch (err) {
        logUi("warn", "history read failed", { error: String(err) });
        setError(String(err));
      } finally {
        setLoading(false);
      }
    },
    [streamId],
  );

  useEffect(() => {
    void load(false);
  }, [load]);
  // Silent: the list doesn't flash a spinner every time the agent commits.
  useRerunOnChange(log?.reads ?? NO_READS, () => void load(true));

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
    for (const commit of log.commits) set.add(commit.author);
    return [...set].sort((a, b) => a.localeCompare(b));
  }, [log]);

  const reachableShas = useMemo(() => reachableFromBranch(log, branch), [log, branch]);

  const visibleCommits = useMemo(() => {
    if (!log) return [] as RevisionInfo[];
    if (!reachableShas) return log.commits;
    return log.commits.filter((c) => reachableShas.has(c.id));
  }, [log, reachableShas]);

  const queryLower = query.trim().toLowerCase();
  const matches = useMemo(() => {
    if (!queryLower && !author) return null;
    const out = new Set<string>();
    for (const commit of visibleCommits) {
      if (author && commit.author !== author) continue;
      if (!queryLower) { out.add(commit.id); continue; }
      const hit = commit.id.toLowerCase().includes(queryLower)
        || commit.subject.toLowerCase().includes(queryLower)
        || commit.author.toLowerCase().includes(queryLower)
        || commit.email.toLowerCase().includes(queryLower);
      if (hit) out.add(commit.id);
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
          <div style={{ padding: 12, color: "#ff6b6b", fontSize: "var(--text-xs)" }}>{error}</div>
        ) : !stream ? (
          <div style={{ padding: 12, color: "var(--muted)", fontSize: "var(--text-xs)" }}>No stream selected.</div>
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

function reachableFromBranch(log: History | null, branch: string): Set<string> | null {
  if (!log || !branch) return null;
  const head = log.branchHeads.find((b) => b.name === branch);
  if (!head) return new Set();
  const parentsBySha = new Map<string, string[]>();
  for (const commit of log.commits) parentsBySha.set(commit.id, commit.parents);
  const reachable = new Set<string>();
  const stack = [head.sha];
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
  fontSize: "var(--text-xs)",
};

const listStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflow: "auto",
  background: "var(--bg)",
};
