import { useEffect, useState } from "react";
import type { AgentStatus, Batch, BatchWorkState, WorkItem } from "../../api.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import {
  batchInputStyle,
  batchStatusColor,
  iconButtonStyle,
  InlineBadge,
  Row,
  smallButtonStyle,
} from "./shared.js";

export function BatchQueueSection({
  batches,
  batchWorkStates,
  agentStatuses,
  selectedBatchId,
  activeBatchId,
  onSelectBatch,
  onCreateBatch,
  onReorderBatch,
  onPromoteBatch,
  onCompleteBatch,
}: {
  batches: Batch[];
  batchWorkStates: Record<string, BatchWorkState>;
  agentStatuses: Record<string, AgentStatus>;
  selectedBatchId: string | null;
  activeBatchId: string | null;
  onSelectBatch(batchId: string): Promise<void>;
  onCreateBatch(title: string): Promise<void>;
  onReorderBatch(batchId: string, targetIndex: number): Promise<void>;
  onPromoteBatch(batchId: string): Promise<void>;
  onCompleteBatch(batchId: string): Promise<void>;
}) {
  const hasQueued = batches.some((batch) => batch.status === "queued");
  const [showCreate, setShowCreate] = useState(false);
  const [title, setTitle] = useState(`Batch ${batches.length + 1}`);
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [expandedBatchIds, setExpandedBatchIds] = useState<Record<string, boolean>>({});

  useEffect(() => {
    setExpandedBatchIds((prev) => {
      const batchIds = new Set(batches.map((batch) => batch.id));
      const next: Record<string, boolean> = {};
      for (const [id, expanded] of Object.entries(prev)) {
        if (batchIds.has(id)) next[id] = expanded;
      }
      return next;
    });
  }, [batches]);

  useEffect(() => {
    if (!selectedBatchId) return;
    setExpandedBatchIds((prev) => (prev[selectedBatchId] ? prev : { ...prev, [selectedBatchId]: true }));
  }, [selectedBatchId]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, minWidth: 260 }}>
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {batches.map((batch) => {
          const active = batch.id === activeBatchId;
          const selectedRow = batch.id === selectedBatchId;
          const expanded = !!expandedBatchIds[batch.id];
          const workState = batchWorkStates[batch.id];
          const workCount = workState?.items.length ?? 0;
          const waitingCount = workState?.waiting.length ?? 0;
          const progressCount = workState?.inProgress.length ?? 0;
          const doneCount = workState?.done.length ?? 0;
          return (
            <div
              key={batch.id}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "stretch",
                gap: 4,
                padding: "8px 10px",
                border: "1px solid var(--border)",
                borderRadius: 6,
                background: selectedRow ? "rgba(74, 158, 255, 0.12)" : "var(--bg-2)",
                color: "inherit",
                textAlign: "left",
                fontFamily: "inherit",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <button
                  onClick={() => void onSelectBatch(batch.id)}
                  style={{
                    flex: 1,
                    display: "flex",
                    justifyContent: "space-between",
                    gap: 8,
                    border: "none",
                    background: "transparent",
                    color: "inherit",
                    cursor: "pointer",
                    textAlign: "left",
                    padding: 0,
                    font: "inherit",
                  }}
                >
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 6, fontWeight: 600 }}>
                    <AgentStatusDot status={agentStatuses[batch.id] ?? "idle"} />
                    {batch.title}
                  </span>
                  <span style={{ color: batchStatusColor(batch.status) }}>{active ? "active" : batch.status}</span>
                </button>
                <button
                  onClick={() => {
                    setExpandedBatchIds((prev) => ({ ...prev, [batch.id]: !prev[batch.id] }));
                  }}
                  aria-label={expanded ? `Collapse ${batch.title}` : `Expand ${batch.title}`}
                  style={iconButtonStyle}
                >
                  {expanded ? "▾" : "▸"}
                </button>
              </div>
              {expanded ? (
                <div style={{ display: "flex", flexDirection: "column", gap: 8, paddingTop: 6, borderTop: "1px solid var(--border)" }}>
                  <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                    <InlineBadge>{`#${batch.sort_index + 1}`}</InlineBadge>
                    <InlineBadge>{`${workCount} items`}</InlineBadge>
                    <InlineBadge>{`${waitingCount} waiting`}</InlineBadge>
                    <InlineBadge>{`${progressCount} in progress`}</InlineBadge>
                    <InlineBadge>{`${doneCount} done`}</InlineBadge>
                  </div>
                  <Row label="Resume" value={batch.resume_session_id || "not started yet"} />
                  <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                    <button
                      onClick={() => void onReorderBatch(batch.id, batch.sort_index - 1)}
                      style={smallButtonStyle}
                      disabled={batch.sort_index === 0}
                    >
                      Move up
                    </button>
                    <button
                      onClick={() => void onReorderBatch(batch.id, batch.sort_index + 1)}
                      style={smallButtonStyle}
                      disabled={batch.sort_index >= batches.length - 1}
                    >
                      Move down
                    </button>
                    {batch.id !== activeBatchId && batch.status !== "completed" ? (
                      <button onClick={() => void onPromoteBatch(batch.id)} style={smallButtonStyle}>
                        Move to top
                      </button>
                    ) : null}
                    {batch.id === activeBatchId ? (
                      <button onClick={() => void onCompleteBatch(batch.id)} style={smallButtonStyle} disabled={!hasQueued}>
                        Complete batch
                      </button>
                    ) : null}
                  </div>
                  {batch.id !== activeBatchId && batch.status !== "completed" ? (
                    <div style={{ color: "var(--muted)", fontSize: 11 }}>
                      Queued batches are for questions and planning until promoted.
                    </div>
                  ) : null}
                  <BatchWorkSummary workState={workState} />
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <button
          onClick={() => {
            setShowCreate((current) => !current);
            setTitle((current) => current || `Batch ${batches.length + 1}`);
            setFormError(null);
          }}
          style={smallButtonStyle}
        >
          {showCreate ? "Cancel" : "+ Batch"}
        </button>
        {showCreate ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 8,
              padding: 10,
              border: "1px solid var(--border)",
              borderRadius: 6,
              background: "var(--bg-2)",
            }}
          >
            <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              <span style={{ color: "var(--muted)", fontSize: 11 }}>Title</span>
              <input
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                style={batchInputStyle}
                placeholder="Batch title"
              />
            </label>
            {formError ? <div style={{ color: "#ff6b6b", fontSize: 11 }}>{formError}</div> : null}
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
              <button
                onClick={() => {
                  setShowCreate(false);
                  setFormError(null);
                }}
                style={smallButtonStyle}
                disabled={submitting}
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  const nextTitle = title.trim();
                  if (!nextTitle) {
                    setFormError("Title is required");
                    return;
                  }
                  setSubmitting(true);
                  setFormError(null);
                  void onCreateBatch(nextTitle)
                    .then(() => {
                      setShowCreate(false);
                      setTitle(`Batch ${batches.length + 2}`);
                    })
                    .catch((error) => {
                      setFormError(String(error));
                    })
                    .finally(() => {
                      setSubmitting(false);
                    });
                }}
                style={smallButtonStyle}
                disabled={submitting}
              >
                {submitting ? "Creating…" : "Create"}
              </button>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function BatchWorkSummary({ workState }: { workState?: BatchWorkState }) {
  if (!workState) {
    return <div style={{ color: "var(--muted)", fontSize: 11 }}>Loading work items…</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>
        Work items
      </div>
      {workState.items.length === 0 ? (
        <div style={{ color: "var(--muted)", fontSize: 11 }}>No work items yet.</div>
      ) : (
        <>
          <WorkBucket title="In progress" items={workState.inProgress} />
          <WorkBucket title="Waiting" items={workState.waiting} />
          <WorkBucket title="Done" items={workState.done} />
        </>
      )}
    </div>
  );
}

function WorkBucket({ title, items }: { title: string; items: WorkItem[] }) {
  if (items.length === 0) return null;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ color: "var(--muted)", fontSize: 10, textTransform: "uppercase", letterSpacing: 0.6 }}>{title}</div>
      {items.map((item) => (
        <div
          key={item.id}
          style={{
            display: "flex",
            justifyContent: "space-between",
            gap: 8,
            padding: "6px 8px",
            borderRadius: 6,
            background: "rgba(255, 255, 255, 0.03)",
          }}
        >
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 12, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
              {item.title}
            </div>
            <div style={{ color: "var(--muted)", fontSize: 10 }}>
              {item.kind}{item.parent_id ? " · linked" : ""}
            </div>
          </div>
          <InlineBadge>{item.priority}</InlineBadge>
        </div>
      ))}
    </div>
  );
}
