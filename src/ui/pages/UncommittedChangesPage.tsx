import { useCallback, useEffect, useMemo, useState } from "react";
import type { BranchChangeEntry, BranchChanges, GitFileStatus, Stream } from "../api.js";
import {
  getBranchChanges,
  gitCommitAll,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { indexRef } from "../tabs/pageRefs.js";

export interface UncommittedChangesPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

interface DirNode {
  name: string;
  path: string;
  files: BranchChangeEntry[];
  children: Map<string, DirNode>;
  totalAdditions: number;
  totalDeletions: number;
  totalFiles: number;
}

const STATUS_LABELS: Record<GitFileStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
};

export function UncommittedChangesPage({ stream, onOpenPage, onOpenFile }: UncommittedChangesPageProps) {
  const streamId = stream?.id ?? null;
  const [data, setData] = useState<BranchChanges | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set([""]));
  const [committing, setCommitting] = useState(false);

  const refresh = useCallback(async () => {
    if (!streamId) {
      setData(null);
      setLoading(false);
      return;
    }
    try {
      setError(null);
      const result = await getBranchChanges(streamId, "HEAD");
      setData(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [streamId]);

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

  const summary = useMemo(() => summarize(data?.files ?? []), [data]);
  const tree = useMemo(() => buildTree(data?.files ?? []), [data]);

  const onCommitAll = useCallback(async () => {
    if (!streamId) return;
    const message = window.prompt("Commit message:");
    if (!message || !message.trim()) return;
    setCommitting(true);
    try {
      const result = await gitCommitAll(streamId, message.trim());
      if (!result.ok) {
        window.alert(`Commit failed:\n${result.stderr || "git error"}`);
      } else {
        await refresh();
      }
    } finally {
      setCommitting(false);
    }
  }, [streamId, refresh]);

  const toggleDir = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  if (!streamId) {
    return (
      <Page testId="page-uncommitted-changes" title="Uncommitted changes" kind="git">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-uncommitted-changes" title="Uncommitted changes" kind="git">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {error ? <div style={errorBanner}>{error}</div> : null}
        {loading && !data ? <div style={muted}>Loading…</div> : null}

        {data ? (
          <>
            <section data-testid="uncommitted-summary" style={card}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
                <div style={{ fontWeight: 600 }}>Summary</div>
                <button
                  type="button"
                  data-testid="uncommitted-commit-all"
                  onClick={onCommitAll}
                  disabled={committing || summary.total === 0}
                  style={primaryButton}
                >
                  {committing ? "Committing…" : "Commit all…"}
                </button>
              </div>
              <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
                <Stat label="Total files" value={summary.total} />
                <Stat label="Modified" value={summary.modified} />
                <Stat label="Added" value={summary.added} />
                <Stat label="Deleted" value={summary.deleted} />
                <Stat label="Renamed" value={summary.renamed} />
                <Stat label="Untracked" value={summary.untracked} />
                <Stat label="+ lines" value={summary.additions} color="var(--text-success, #16a34a)" />
                <Stat label="− lines" value={summary.deletions} color="var(--text-danger, #dc2626)" />
              </div>
            </section>

            <section data-testid="uncommitted-tree" style={card}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
                <div style={{ fontWeight: 600 }}>Changed files</div>
                <button
                  type="button"
                  data-testid="uncommitted-open-files-page"
                  onClick={() => onOpenPage(indexRef("files"))}
                  style={linkButton}
                >
                  Open file tree →
                </button>
              </div>
              {summary.total === 0 ? (
                <div style={muted}>Working tree is clean.</div>
              ) : (
                <DirTreeView
                  node={tree}
                  expanded={expanded}
                  toggle={toggleDir}
                  onOpenFile={onOpenFile}
                  depth={0}
                />
              )}
            </section>
          </>
        ) : null}
      </div>
    </Page>
  );
}

function DirTreeView({
  node,
  expanded,
  toggle,
  onOpenFile,
  depth,
}: {
  node: DirNode;
  expanded: Set<string>;
  toggle(path: string): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  depth: number;
}) {
  const isExpanded = expanded.has(node.path);
  const children = [...node.children.values()].sort((a, b) => a.name.localeCompare(b.name));
  const files = [...node.files].sort((a, b) => a.path.localeCompare(b.path));
  const showHeader = depth > 0 || node.path !== "";
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      {showHeader ? (
        <div
          data-testid="uncommitted-tree-folder"
          onClick={() => toggle(node.path)}
          style={{
            display: "flex",
            gap: 8,
            alignItems: "center",
            paddingLeft: depth * 12,
            cursor: "pointer",
            userSelect: "none",
          }}
        >
          <span style={{ width: 12 }}>{isExpanded ? "▾" : "▸"}</span>
          <span style={{ flex: 1, fontWeight: 500 }}>{node.name || "/"}</span>
          <span style={subtle}>
            {node.totalFiles} file{node.totalFiles === 1 ? "" : "s"}
          </span>
          <span style={addCol}>+{node.totalAdditions}</span>
          <span style={delCol}>−{node.totalDeletions}</span>
        </div>
      ) : null}
      {(!showHeader || isExpanded) && (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {children.map((child) => (
            <DirTreeView
              key={child.path}
              node={child}
              expanded={expanded}
              toggle={toggle}
              onOpenFile={onOpenFile}
              depth={depth + (showHeader ? 1 : 0)}
            />
          ))}
          {files.map((file) => (
            <div
              key={file.path}
              data-testid="uncommitted-tree-file"
              onClick={(e) => onOpenFile(file.path, { newTab: e.metaKey || e.ctrlKey })}
              onAuxClick={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  onOpenFile(file.path, { newTab: true });
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                onOpenFile(file.path, { newTab: true });
              }}
              style={{
                display: "flex",
                gap: 8,
                alignItems: "center",
                paddingLeft: (depth + (showHeader ? 1 : 0)) * 12 + 12,
                cursor: "pointer",
                fontSize: 13,
              }}
            >
              <span style={{ width: 14, color: "var(--text-muted)" }}>{STATUS_LABELS[file.status]}</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {basename(file.path)}
              </span>
              <span style={addCol}>{file.additions == null ? "" : `+${file.additions}`}</span>
              <span style={delCol}>{file.deletions == null ? "" : `−${file.deletions}`}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, color }: { label: string; value: number; color?: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", minWidth: 70 }}>
      <span style={{ fontSize: 11, color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: 0.5 }}>
        {label}
      </span>
      <span style={{ fontSize: 18, fontWeight: 600, color: color ?? "var(--text-primary)" }}>{value}</span>
    </div>
  );
}

interface SummaryNumbers {
  total: number;
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  additions: number;
  deletions: number;
}

export function summarize(files: BranchChangeEntry[]): SummaryNumbers {
  const out: SummaryNumbers = {
    total: files.length,
    modified: 0,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
    additions: 0,
    deletions: 0,
  };
  for (const file of files) {
    out[file.status] += 1;
    out.additions += file.additions ?? 0;
    out.deletions += file.deletions ?? 0;
  }
  return out;
}

export function buildTree(files: BranchChangeEntry[]): DirNode {
  const root: DirNode = makeNode("", "");
  for (const file of files) {
    const parts = file.path.split("/");
    let cursor = root;
    cursor.totalAdditions += file.additions ?? 0;
    cursor.totalDeletions += file.deletions ?? 0;
    cursor.totalFiles += 1;
    for (let i = 0; i < parts.length - 1; i++) {
      const segment = parts[i]!;
      const childPath = cursor.path === "" ? segment : `${cursor.path}/${segment}`;
      let child = cursor.children.get(segment);
      if (!child) {
        child = makeNode(segment, childPath);
        cursor.children.set(segment, child);
      }
      child.totalAdditions += file.additions ?? 0;
      child.totalDeletions += file.deletions ?? 0;
      child.totalFiles += 1;
      cursor = child;
    }
    cursor.files.push(file);
  }
  return root;
}

function makeNode(name: string, path: string): DirNode {
  return {
    name,
    path,
    files: [],
    children: new Map(),
    totalAdditions: 0,
    totalDeletions: 0,
    totalFiles: 0,
  };
}

function basename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx < 0 ? path : path.slice(idx + 1);
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 13 };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const addCol: React.CSSProperties = { color: "var(--text-success, #16a34a)", width: 48, textAlign: "right", fontSize: 12 };
const delCol: React.CSSProperties = { color: "var(--text-danger, #dc2626)", width: 48, textAlign: "right", fontSize: 12 };
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
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontSize: 12,
  cursor: "pointer",
};
