import { useMemo, useState } from "react";
import type { AgentTurn, BatchFileChange, WorkItem } from "../../api.js";
import { netFileChanges, fileChangeKindColor } from "../../file-change-net.js";

interface Props {
  agentTurns: AgentTurn[] | null;
  batchFileChanges: BatchFileChange[] | null;
  workItems: WorkItem[];
  onOpenFile(path: string): void;
  onOpenTurnDiff?(turnId: string, path: string): void;
}

export function Activity({ agentTurns, batchFileChanges, workItems, onOpenFile, onOpenTurnDiff }: Props) {
  const itemById = useMemo(() => new Map(workItems.map((item) => [item.id, item] as const)), [workItems]);
  const changesByTurn = useMemo(() => {
    const out = new Map<string, BatchFileChange[]>();
    for (const change of batchFileChanges ?? []) {
      if (!change.turn_id) continue;
      const list = out.get(change.turn_id) ?? [];
      list.push(change);
      out.set(change.turn_id, list);
    }
    return out;
  }, [batchFileChanges]);

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
          linkedItem={turn.work_item_id ? itemById.get(turn.work_item_id) ?? null : null}
          fileChanges={changesByTurn.get(turn.id) ?? []}
          onOpenFile={onOpenFile}
          onOpenTurnDiff={onOpenTurnDiff}
        />
      ))}
    </div>
  );
}

function TurnCard({
  turn,
  linkedItem,
  fileChanges,
  onOpenFile,
  onOpenTurnDiff,
}: {
  turn: AgentTurn;
  linkedItem: WorkItem | null;
  fileChanges: BatchFileChange[];
  onOpenFile(path: string): void;
  onOpenTurnDiff?(turnId: string, path: string): void;
}) {
  const open = turn.ended_at == null;
  const [filesOpen, setFilesOpen] = useState(false);
  const deduped = netFileChanges(fileChanges);
  return (
    <div style={{ border: "1px solid var(--border)", borderRadius: 8, background: "var(--bg-2)", padding: 10, display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline", fontSize: 11, color: "var(--muted)" }}>
        <span>{new Date(turn.started_at).toLocaleString()}</span>
        {open ? <span style={{ color: "var(--accent)" }}>· in progress</span> : null}
        {linkedItem ? (
          <span style={{ marginLeft: "auto", padding: "1px 6px", border: "1px solid var(--border)", borderRadius: 999 }}>
            {linkedItem.title}
          </span>
        ) : null}
      </div>
      <div style={{ fontSize: 13, whiteSpace: "pre-wrap", overflow: "hidden", display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical" }}>
        {turn.prompt}
      </div>
      {turn.answer ? (
        <div style={{ fontSize: 12, color: "var(--muted)", whiteSpace: "pre-wrap", overflow: "hidden", display: "-webkit-box", WebkitLineClamp: 3, WebkitBoxOrient: "vertical" }}>
          ↳ {turn.answer}
        </div>
      ) : null}
      {deduped.length > 0 ? (
        <div style={{ marginTop: 4 }}>
          <button
            onClick={() => setFilesOpen((v) => !v)}
            style={{ background: "transparent", border: "none", color: "var(--muted)", cursor: "pointer", padding: 0, fontSize: 11, fontFamily: "inherit" }}
          >
            {filesOpen ? "▾" : "▸"} {deduped.length} file{deduped.length === 1 ? "" : "s"} touched
          </button>
          {filesOpen ? (
            <div style={{ marginTop: 4, display: "flex", flexDirection: "column", gap: 2 }}>
              {deduped.map((change) => (
                <button
                  key={change.id}
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
                      color: fileChangeKindColor(change.change_kind),
                      minWidth: 52,
                      textAlign: "center",
                    }}
                  >
                    {change.change_kind}
                  </span>
                  <span style={{ fontFamily: "ui-monospace, monospace", textDecoration: "underline", textDecorationStyle: "dotted", textUnderlineOffset: 2 }}>{change.path}</span>
                  {onOpenTurnDiff ? (
                    <span
                      role="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        onOpenTurnDiff(turn.id, change.path);
                      }}
                      title="Open turn diff"
                      style={{ fontSize: 10, padding: "0 4px", border: "1px solid var(--border)", borderRadius: 3, cursor: "pointer", color: "var(--muted)" }}
                    >
                      diff
                    </span>
                  ) : null}
                  <span style={{ color: "var(--muted)", marginLeft: "auto" }}>
                    {change.source}
                    {change.tool_name ? ` · ${change.tool_name}` : ""}
                  </span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
