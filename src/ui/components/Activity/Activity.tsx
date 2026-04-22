import { useEffect, useState } from "react";
import type { AgentTurn, WorkItem } from "../../api.js";
import { getSnapshotSummary } from "../../api.js";

interface Props {
  agentTurns: AgentTurn[] | null;
  workItems: WorkItem[];
  onOpenFile(path: string): void;
  onOpenTurnDiff?(turn: AgentTurn, path: string): void;
}

export function Activity({ agentTurns, onOpenFile, onOpenTurnDiff }: Props) {
  if (agentTurns === null) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Loading…</div>;
  }
  if (agentTurns.length === 0) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No agent turns yet.</div>;
  }
  return (
    <div style={{ flex: 1, overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 8 }}>
      {agentTurns.map((turn) => (
        <TurnCard
          key={turn.id}
          turn={turn}
          onOpenFile={onOpenFile}
          onOpenTurnDiff={onOpenTurnDiff}
        />
      ))}
    </div>
  );
}

function TokenBadge({ turn }: { turn: AgentTurn }) {
  const { input_tokens, output_tokens, cache_read_input_tokens } = turn;
  if (input_tokens === null && output_tokens === null && cache_read_input_tokens === null) {
    return null;
  }
  const parts: string[] = [];
  if (input_tokens !== null) parts.push(`${formatTokens(input_tokens)} in`);
  if (output_tokens !== null) parts.push(`${formatTokens(output_tokens)} out`);
  if (cache_read_input_tokens !== null && cache_read_input_tokens > 0) {
    parts.push(`${formatTokens(cache_read_input_tokens)} cache`);
  }
  return (
    <span
      style={{ fontSize: 10, fontFamily: "ui-monospace, monospace", color: "var(--muted)" }}
      title={`Token usage for this turn — input: ${input_tokens ?? 0}, output: ${output_tokens ?? 0}, cache-read: ${cache_read_input_tokens ?? 0}`}
    >
      · {parts.join(" / ")}
    </span>
  );
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

interface TurnChange {
  path: string;
  kind: "created" | "updated" | "deleted";
}

function TurnCard({
  turn,
  onOpenFile,
  onOpenTurnDiff,
}: {
  turn: AgentTurn;
  onOpenFile(path: string): void;
  onOpenTurnDiff?(turn: AgentTurn, path: string): void;
}) {
  const open = turn.ended_at == null;
  const [filesOpen, setFilesOpen] = useState(false);
  const [changes, setChanges] = useState<TurnChange[]>([]);

  useEffect(() => {
    // Build the per-turn changed-files list by diffing the turn's
    // start and end snapshots. Nothing to show until both exist.
    if (!turn.start_snapshot_id || !turn.end_snapshot_id) {
      setChanges([]);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        // Explicit baseline: this turn's start snapshot, not "whatever
        // snapshot happened to land most recently in the stream." Without
        // this, a turn that didn't change any files would attribute another
        // turn's or task's changes to itself because the dedup'd
        // end_snapshot_id equals some older snapshot whose implicit
        // "previous" is a sibling, not turn-local.
        const summary = await getSnapshotSummary(turn.end_snapshot_id!, turn.start_snapshot_id);
        if (cancelled || !summary) return;
        const list: TurnChange[] = Object.entries(summary.files).map(([path, row]) => ({
          path,
          kind: row.kind,
        }));
        list.sort((a, b) => a.path.localeCompare(b.path));
        setChanges(list);
      } catch {
        if (!cancelled) setChanges([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [turn.start_snapshot_id, turn.end_snapshot_id]);

  return (
    <div style={{ border: "1px solid var(--border)", borderRadius: 8, background: "var(--bg-2)", padding: 10, display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline", fontSize: 11, color: "var(--muted)" }}>
        <span>{new Date(turn.started_at).toLocaleString()}</span>
        {open ? <span style={{ color: "var(--accent)" }}>· in progress</span> : null}
        <TokenBadge turn={turn} />
      </div>
      <div style={{ fontSize: 13, whiteSpace: "pre-wrap", overflow: "hidden", display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical" }}>
        {turn.prompt}
      </div>
      {turn.answer ? (
        <div style={{ fontSize: 12, color: "var(--muted)", whiteSpace: "pre-wrap", overflow: "hidden", display: "-webkit-box", WebkitLineClamp: 3, WebkitBoxOrient: "vertical" }}>
          ↳ {turn.answer}
        </div>
      ) : null}
      {changes.length > 0 ? (
        <div style={{ marginTop: 4 }}>
          <button type="button"
            onClick={() => setFilesOpen((v) => !v)}
            style={{ background: "transparent", border: "none", color: "var(--muted)", cursor: "pointer", padding: 0, fontSize: 11, fontFamily: "inherit" }}
          >
            {filesOpen ? "▾" : "▸"} {changes.length} file{changes.length === 1 ? "" : "s"} touched
          </button>
          {filesOpen ? (
            <div style={{ marginTop: 4, display: "flex", flexDirection: "column", gap: 2 }}>
              {changes.map((change) => (
                <button type="button"
                  key={change.path}
                  onClick={() => onOpenFile(change.path)}
                  style={{ background: "transparent", border: "none", padding: 0, fontSize: 11, display: "flex", gap: 6, alignItems: "baseline", cursor: "pointer", textAlign: "left", color: "inherit", fontFamily: "inherit" }}
                  title={`Open ${change.path}`}
                >
                  <span
                    style={{
                      fontSize: 9,
                      textTransform: "uppercase",
                      padding: "0 4px",
                      borderRadius: 3,
                      background: "var(--bg)",
                      border: "1px solid var(--border)",
                      color: fileChangeKindColor(change.kind),
                      minWidth: 52,
                      textAlign: "center",
                    }}
                  >
                    {change.kind}
                  </span>
                  <span style={{ fontFamily: "ui-monospace, monospace", textDecoration: "underline", textDecorationStyle: "dotted", textUnderlineOffset: 2 }}>{change.path}</span>
                  {onOpenTurnDiff ? (
                    <span
                      role="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        onOpenTurnDiff(turn, change.path);
                      }}
                      title="Open turn diff"
                      style={{ fontSize: 10, padding: "0 4px", border: "1px solid var(--border)", borderRadius: 3, cursor: "pointer", color: "var(--muted)" }}
                    >
                      diff
                    </span>
                  ) : null}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function fileChangeKindColor(kind: "created" | "updated" | "deleted"): string {
  switch (kind) {
    case "created": return "var(--accent)";
    case "deleted": return "#d66";
    default: return "var(--fg)";
  }
}
