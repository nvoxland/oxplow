import { useEffect, useMemo, useState } from "react";
import {
  listCodeQualityFindings,
  listCodeQualityScans,
  runCodeQualityScan,
  subscribeCodeQualityEvents,
  type CodeQualityFindingRow,
  type CodeQualityScanRow,
  type CodeQualityScope,
  type CodeQualityTool,
  type Stream,
} from "../../api.js";

interface Props {
  stream: Stream | null;
  onOpenFile?: (path: string) => void;
}

const TOOLS: CodeQualityTool[] = ["lizard", "jscpd"];

export function CodeQualityPanel({ stream, onOpenFile }: Props) {
  const [activeScope, setActiveScope] = useState<CodeQualityScope>("codebase");
  const [activeTool, setActiveTool] = useState<CodeQualityTool | "all">("all");
  const [findings, setFindings] = useState<CodeQualityFindingRow[]>([]);
  const [scans, setScans] = useState<CodeQualityScanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [running, setRunning] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stream) {
      setFindings([]);
      setScans([]);
      return;
    }
    let cancelled = false;
    const refetch = () => {
      setLoading(true);
      Promise.all([
        listCodeQualityFindings({ streamId: stream.id }),
        listCodeQualityScans({ streamId: stream.id, limit: 50 }),
      ])
        .then(([f, s]) => {
          if (cancelled) return;
          setFindings(f);
          setScans(s);
          setError(null);
        })
        .catch((err) => {
          if (cancelled) return;
          setError(String(err));
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    };
    refetch();
    const unsub = subscribeCodeQualityEvents(stream.id, (event) => {
      setRunning((prev) => {
        const key = `${event.tool}:${event.scope}`;
        const next = { ...prev };
        if (event.status === "running") next[key] = true;
        else delete next[key];
        return next;
      });
      refetch();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [stream?.id]);

  const visibleFindings = useMemo(() => {
    return findings.filter((f) => {
      const scan = scans.find((s) => s.id === f.scanId);
      if (!scan) return false;
      if (scan.scope !== activeScope) return false;
      if (activeTool !== "all" && scan.tool !== activeTool) return false;
      return true;
    });
  }, [findings, scans, activeScope, activeTool]);

  const findingsByFile = useMemo(() => {
    const map = new Map<string, CodeQualityFindingRow[]>();
    for (const f of visibleFindings) {
      const arr = map.get(f.path) ?? [];
      arr.push(f);
      map.set(f.path, arr);
    }
    for (const arr of map.values()) {
      arr.sort((a, b) => b.metricValue - a.metricValue);
    }
    return Array.from(map.entries()).sort((a, b) => {
      const aMax = a[1][0]?.metricValue ?? 0;
      const bMax = b[1][0]?.metricValue ?? 0;
      return bMax - aMax;
    });
  }, [visibleFindings]);

  const visibleScans = useMemo(
    () => scans.filter((s) => s.scope === activeScope),
    [scans, activeScope],
  );

  async function handleRun(tool: CodeQualityTool) {
    if (!stream) return;
    const key = `${tool}:${activeScope}`;
    setRunning((prev) => ({ ...prev, [key]: true }));
    setError(null);
    try {
      await runCodeQualityScan({
        streamId: stream.id,
        tool,
        scope: activeScope,
      });
    } catch (err) {
      setError(String(err));
    } finally {
      setRunning((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
    }
  }

  if (!stream) {
    return (
      <div style={{ padding: 12, color: "var(--muted-foreground)" }}>
        Select a stream to run code quality scans.
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 12px",
          borderBottom: "1px solid var(--border)",
          flexShrink: 0,
          flexWrap: "wrap",
        }}
      >
        <ScopeTabs active={activeScope} onChange={setActiveScope} />
        <ToolFilter active={activeTool} onChange={setActiveTool} />
        <div style={{ flex: 1 }} />
        {TOOLS.map((tool) => {
          const key = `${tool}:${activeScope}`;
          const isRunning = !!running[key];
          return (
            <button
              key={tool}
              type="button"
              disabled={isRunning}
              onClick={() => void handleRun(tool)}
              style={{ padding: "4px 10px" }}
              title={`Run ${tool} for ${activeScope === "codebase" ? "the whole codebase" : "files changed vs base ref"}`}
            >
              {isRunning ? `Running ${tool}…` : `Run ${tool}`}
            </button>
          );
        })}
      </div>
      {error ? (
        <div style={{ padding: "6px 12px", color: "var(--danger)", fontSize: 12 }}>{error}</div>
      ) : null}
      <ScanStatusStrip scans={visibleScans} />
      <div style={{ flex: 1, overflowY: "auto" }}>
        {loading && findings.length === 0 ? (
          <div style={{ padding: 12, color: "var(--muted-foreground)" }}>Loading…</div>
        ) : findingsByFile.length === 0 ? (
          <div style={{ padding: 12, color: "var(--muted-foreground)" }}>
            {activeScope === "diff"
              ? "No findings for changed files yet. Run a scan to populate."
              : "No findings yet. Run a scan to populate."}
          </div>
        ) : (
          findingsByFile.map(([path, rows]) => (
            <FileGroup
              key={path}
              path={path}
              rows={rows}
              onOpen={() => onOpenFile?.(path)}
            />
          ))
        )}
      </div>
    </div>
  );
}

function ScopeTabs({
  active,
  onChange,
}: {
  active: CodeQualityScope;
  onChange: (scope: CodeQualityScope) => void;
}) {
  return (
    <div style={{ display: "flex", gap: 2 }}>
      {(["codebase", "diff"] as const).map((s) => (
        <button
          key={s}
          type="button"
          onClick={() => onChange(s)}
          style={{
            padding: "4px 10px",
            background: active === s ? "var(--accent)" : "transparent",
            color: active === s ? "var(--accent-foreground)" : "inherit",
            border: "1px solid var(--border)",
            borderRadius: 4,
          }}
        >
          {s === "codebase" ? "Whole codebase" : "Diff vs base"}
        </button>
      ))}
    </div>
  );
}

function ToolFilter({
  active,
  onChange,
}: {
  active: CodeQualityTool | "all";
  onChange: (tool: CodeQualityTool | "all") => void;
}) {
  return (
    <select
      value={active}
      onChange={(e) => onChange(e.target.value as CodeQualityTool | "all")}
      style={{ padding: "3px 6px" }}
    >
      <option value="all">all tools</option>
      <option value="lizard">lizard</option>
      <option value="jscpd">jscpd</option>
    </select>
  );
}

function ScanStatusStrip({ scans }: { scans: CodeQualityScanRow[] }) {
  if (scans.length === 0) return null;
  const latestByTool = new Map<string, CodeQualityScanRow>();
  for (const s of scans) {
    const existing = latestByTool.get(s.tool);
    if (!existing || existing.started_at < s.started_at) latestByTool.set(s.tool, s);
  }
  const rows = Array.from(latestByTool.values());
  return (
    <div
      style={{
        display: "flex",
        gap: 12,
        padding: "4px 12px",
        fontSize: 11,
        color: "var(--muted-foreground)",
        borderBottom: "1px solid var(--border)",
        flexShrink: 0,
        flexWrap: "wrap",
      }}
    >
      {rows.map((s) => (
        <span key={s.id} title={s.error_message ?? undefined}>
          {s.tool}: {s.status}
          {s.status === "failed" && s.error_message ? ` — ${s.error_message}` : ""}
        </span>
      ))}
    </div>
  );
}

const KIND_LABEL: Record<string, string> = {
  complexity: "CCN",
  "function-length": "len",
  "parameter-count": "params",
  "duplicate-block": "dup-lines",
};

function FileGroup({
  path,
  rows,
  onOpen,
}: {
  path: string;
  rows: CodeQualityFindingRow[];
  onOpen: () => void;
}) {
  return (
    <div style={{ borderBottom: "1px solid var(--border)" }}>
      <button
        type="button"
        onClick={onOpen}
        style={{
          display: "block",
          width: "100%",
          textAlign: "left",
          padding: "6px 12px",
          background: "transparent",
          border: "none",
          cursor: "pointer",
          fontFamily: "var(--font-mono)",
          fontSize: 12,
        }}
        title="Open file"
      >
        {path} <span style={{ color: "var(--muted-foreground)" }}>({rows.length})</span>
      </button>
      <div style={{ padding: "0 24px 8px" }}>
        {rows.map((r) => (
          <FindingRow key={r.id} row={r} />
        ))}
      </div>
    </div>
  );
}

function FindingRow({ row }: { row: CodeQualityFindingRow }) {
  const fnName = (row.extra as { functionName?: string } | null)?.functionName;
  const peer = row.extra as { peerPath?: string; peerStartLine?: number; peerEndLine?: number } | null;
  return (
    <div style={{ display: "flex", gap: 8, fontSize: 11, color: "var(--muted-foreground)" }}>
      <span style={{ minWidth: 80 }}>
        {KIND_LABEL[row.kind] ?? row.kind}: <strong>{row.metricValue}</strong>
      </span>
      <span style={{ minWidth: 90 }}>
        L{row.startLine}-{row.endLine}
      </span>
      {fnName ? <span>{fnName}</span> : null}
      {peer?.peerPath ? (
        <span>
          ↔ {peer.peerPath}:L{peer.peerStartLine}-{peer.peerEndLine}
        </span>
      ) : null}
    </div>
  );
}
