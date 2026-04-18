import { useCallback, useEffect, useMemo, useState } from "react";
import {
  getBranchChanges,
  listWorkspaceFiles,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
  type BranchChanges,
  type GitFileStatus,
  type Stream,
  type WorkspaceIndexedFile,
} from "../../api.js";

export interface DiffRequest {
  path: string;
  leftRef: string;
  rightKind: "working" | { ref: string };
  baseLabel: string;
}

interface Props {
  stream: Stream | null;
  onOpenDiff(request: DiffRequest): void;
}

type Scope = "working" | "branch";

export function GitChangesPanel({ stream, onOpenDiff }: Props) {
  const [scope, setScope] = useState<Scope>("working");
  const [workingFiles, setWorkingFiles] = useState<WorkspaceIndexedFile[] | null>(null);
  const [branch, setBranch] = useState<(BranchChanges & { resolvedBaseRef: string | null }) | null>(null);
  const [baseOverride, setBaseOverride] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadWorking = useCallback(async () => {
    if (!stream) return;
    try {
      const result = await listWorkspaceFiles(stream.id);
      setWorkingFiles(result.files.filter((file) => file.gitStatus !== null));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [stream]);

  const loadBranch = useCallback(async () => {
    if (!stream) return;
    setLoading(true);
    try {
      const result = await getBranchChanges(stream.id, baseOverride || undefined);
      setBranch(result);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [stream, baseOverride]);

  useEffect(() => {
    setWorkingFiles(null);
    setBranch(null);
    setBaseOverride("");
    setError(null);
  }, [stream?.id]);

  useEffect(() => {
    if (scope === "working") void loadWorking();
    else void loadBranch();
  }, [scope, loadWorking, loadBranch]);

  useEffect(() => {
    if (!stream) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        if (scope === "working") void loadWorking();
        else void loadBranch();
      }, 150);
    };
    const unsubscribeWorkspace = subscribeWorkspaceEvents(stream.id, schedule);
    const unsubscribeRefs = subscribeGitRefsEvents(stream.id, schedule);
    return () => {
      unsubscribeWorkspace();
      unsubscribeRefs();
      if (timer) clearTimeout(timer);
    };
  }, [stream, scope, loadWorking, loadBranch]);

  const workingRows = useMemo(
    () =>
      (workingFiles ?? []).map<RowData>((file) => ({
        path: file.path,
        status: file.gitStatus ?? "modified",
        leftRef: "HEAD",
        baseLabel: "HEAD",
        rightKind: "working" as const,
      })),
    [workingFiles],
  );

  const branchRows = useMemo<RowData[]>(() => {
    if (!branch || !branch.mergeBase) return [];
    return branch.files.map((file) => ({
      path: file.path,
      status: file.status,
      additions: file.additions ?? undefined,
      deletions: file.deletions ?? undefined,
      leftRef: branch.mergeBase!,
      baseLabel: branch.resolvedBaseRef ?? branch.baseRef,
      rightKind: "working" as const,
    }));
  }, [branch]);

  const rows = scope === "working" ? workingRows : branchRows;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12 }}>
      <div style={{ display: "flex", gap: 4, padding: 8, borderBottom: "1px solid var(--border)" }}>
        <ScopeToggle scope={scope} onChange={setScope} />
      </div>
      {scope === "branch" ? (
        <div style={{ display: "flex", gap: 6, padding: "6px 8px", borderBottom: "1px solid var(--border)", alignItems: "center" }}>
          <span style={{ color: "var(--muted)", fontSize: 11 }}>vs</span>
          <input
            value={baseOverride || branch?.resolvedBaseRef || ""}
            onChange={(e) => setBaseOverride(e.target.value)}
            onBlur={() => void loadBranch()}
            onKeyDown={(e) => { if (e.key === "Enter") void loadBranch(); }}
            placeholder={branch?.resolvedBaseRef ?? "base branch"}
            style={inputStyle}
          />
          <button onClick={() => void loadBranch()} style={buttonStyle} disabled={loading}>↻</button>
        </div>
      ) : null}
      {error ? <div style={{ padding: "6px 8px", color: "#ff6b6b" }}>{error}</div> : null}
      {scope === "branch" && !loading && branch && !branch.mergeBase ? (
        <div style={{ padding: 8, color: "var(--muted)" }}>
          No base branch found. Default tries origin/main → main → origin/master → master. Override above if needed.
        </div>
      ) : null}
      <div style={{ flex: 1, overflowY: "auto", padding: "4px 0" }}>
        {rows.length === 0 && !loading ? (
          <div style={{ padding: 8, color: "var(--muted)" }}>
            {scope === "working" ? "No uncommitted changes." : "No changes vs branch base."}
          </div>
        ) : null}
        {rows.map((row) => (
          <button
            key={row.path}
            onClick={() =>
              onOpenDiff({
                path: row.path,
                leftRef: row.leftRef,
                rightKind: row.rightKind,
                baseLabel: row.baseLabel,
              })
            }
            style={rowStyle}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-2)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            title={row.path}
          >
            <StatusBadge status={row.status} />
            <span style={{ fontFamily: "ui-monospace, monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, textAlign: "left" }}>
              {row.path}
            </span>
            {row.additions != null || row.deletions != null ? (
              <span style={{ color: "var(--muted)", fontSize: 11, flexShrink: 0 }}>
                {row.additions != null ? <span style={{ color: "var(--accent)" }}>+{row.additions}</span> : null}
                {row.additions != null && row.deletions != null ? " " : null}
                {row.deletions != null ? <span style={{ color: "#d66" }}>−{row.deletions}</span> : null}
              </span>
            ) : null}
          </button>
        ))}
      </div>
    </div>
  );
}

interface RowData {
  path: string;
  status: GitFileStatus;
  additions?: number;
  deletions?: number;
  leftRef: string;
  baseLabel: string;
  rightKind: "working" | { ref: string };
}

function ScopeToggle({ scope, onChange }: { scope: Scope; onChange(next: Scope): void }) {
  return (
    <>
      {(["working", "branch"] as const).map((s) => (
        <button
          key={s}
          onClick={() => onChange(s)}
          style={{
            ...buttonStyle,
            background: scope === s ? "var(--bg)" : "transparent",
            borderColor: scope === s ? "var(--accent)" : "var(--border)",
            color: scope === s ? "var(--fg)" : "var(--muted)",
          }}
        >
          {s === "working" ? "Working copy" : "Branch"}
        </button>
      ))}
    </>
  );
}

function StatusBadge({ status }: { status: GitFileStatus }) {
  const color = statusColor(status);
  return (
    <span
      style={{
        fontSize: 9,
        textTransform: "uppercase",
        padding: "0 4px",
        borderRadius: 3,
        background: "var(--bg-2)",
        border: "1px solid var(--border)",
        color,
        minWidth: 52,
        textAlign: "center",
      }}
    >
      {status}
    </span>
  );
}

function statusColor(status: GitFileStatus): string {
  switch (status) {
    case "added":
    case "untracked":
      return "var(--accent)";
    case "deleted":
      return "#d66";
    case "renamed":
      return "#d9a066";
    default:
      return "var(--fg)";
  }
}

const buttonStyle = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
};

const inputStyle = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 8px",
  borderRadius: 4,
  fontFamily: "inherit",
  fontSize: 11,
  flex: 1,
};

const rowStyle = {
  display: "flex",
  gap: 6,
  alignItems: "center",
  width: "100%",
  padding: "4px 8px",
  background: "transparent",
  border: "none",
  color: "inherit",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
};
