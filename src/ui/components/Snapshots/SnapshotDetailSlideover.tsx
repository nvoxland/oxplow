import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  getSnapshotPairDiff,
  getSnapshotSummary,
  restoreFileFromSnapshot,
  type SnapshotSummary,
  type Stream,
} from "../../api.js";
import { logUi } from "../../logger.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { InlineConfirm } from "../InlineConfirm.js";
import { Slideover } from "../Slideover.js";

/**
 * Pure helper exported for unit tests: chooses a Slideover header label
 * for a snapshot. The detail body shares the same logic the docked
 * SnapshotsPanel uses for its row label, but the slideover header has
 * to render synchronously from the small snapshot identifier the caller
 * already has on hand (label + source) — re-fetching the full
 * SnapshotSummary just for the title would flash a "Loading…" header.
 */
export function buildSnapshotSlideoverTitle(input: { label: string | null; source: string }): string {
  const trimmed = input.label?.trim() ?? "";
  if (trimmed.length > 0) return trimmed;
  switch (input.source) {
    case "task-start":
      return "Task started";
    case "task-end":
      return "Task ended";
    case "task-event":
      return "Task update";
    case "startup":
      return "External changes";
    default:
      return "Snapshot";
  }
}

export interface SnapshotDetailSlideoverProps {
  open: boolean;
  onClose(): void;
  stream: Stream | null;
  /** Snapshot to load. */
  snapshotId: string | null;
  /** Pre-known label/source for an instant header (avoids loading flash). */
  snapshotLabel?: string | null;
  snapshotSource?: string;
  /** Forwarded to the file rows. */
  onOpenDiff?(spec: DiffSpec): void;
  /** Optional: open the work item that wrote this snapshot. */
  workItemId?: string | null;
  onOpenWorkItem?(itemId: string): void;
}

/**
 * Right-edge Slideover wrapper around the snapshot detail body. Used
 * for cross-page opens (e.g. a Backlinks entry that targets a snapshot
 * from the WorkItemPage) — the docked SnapshotsPanel keeps its inline
 * detail layout because the panel already has the horizontal real
 * estate. This component owns its own data fetch so callers only need
 * to pass the snapshot id.
 */
export function SnapshotDetailSlideover({
  open,
  onClose,
  stream,
  snapshotId,
  snapshotLabel = null,
  snapshotSource = "",
  onOpenDiff,
  workItemId = null,
  onOpenWorkItem,
}: SnapshotDetailSlideoverProps) {
  const [summary, setSummary] = useState<SnapshotSummary | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || !snapshotId) {
      setSummary(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void getSnapshotSummary(snapshotId)
      .then((result) => {
        if (cancelled) return;
        setSummary(result);
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot summary failed", { error: String(err) });
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, snapshotId]);

  const title = buildSnapshotSlideoverTitle({ label: snapshotLabel, source: snapshotSource });

  async function handleOpenFileDiff(path: string) {
    if (!stream || !onOpenDiff || !snapshotId || !summary) return;
    try {
      const baseId = summary.previousSnapshotId;
      const result = await getSnapshotPairDiff(baseId, snapshotId, path);
      const label = baseId
        ? `${baseId.slice(-6)} → ${snapshotId.slice(-6)}`
        : `initial → ${snapshotId.slice(-6)}`;
      onOpenDiff({
        path,
        leftRef: "",
        rightKind: "working",
        baseLabel: label,
        leftContent: renderDiffSide(result.before, result.beforeState),
        rightContent: renderDiffSide(result.after, result.afterState),
        labelOverride: label,
      });
    } catch (err) {
      logUi("warn", "open snapshot diff failed", { error: String(err) });
    }
  }

  async function handleRestore(path: string) {
    if (!stream || !snapshotId) return;
    try {
      await restoreFileFromSnapshot(stream.id, snapshotId, path);
    } catch (err) {
      logUi("warn", "restore snapshot file failed", { error: String(err) });
    }
  }

  return (
    <Slideover
      open={open}
      onClose={onClose}
      title={title}
      testId="snapshot-detail-slideover"
    >
      {!snapshotId ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>No snapshot selected.</div>
      ) : loading && !summary ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Loading…</div>
      ) : !summary ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Snapshot not found.</div>
      ) : (
        <SnapshotDetailBody
          summary={summary}
          workItemId={workItemId}
          onOpenWorkItem={onOpenWorkItem}
          onOpenFileDiff={handleOpenFileDiff}
          onRestore={(path) => { void handleRestore(path); }}
        />
      )}
    </Slideover>
  );
}

function SnapshotDetailBody({
  summary,
  workItemId,
  onOpenWorkItem,
  onOpenFileDiff,
  onRestore,
}: {
  summary: SnapshotSummary;
  workItemId: string | null;
  onOpenWorkItem?(itemId: string): void;
  onOpenFileDiff(path: string): void;
  onRestore(path: string): void;
}) {
  const paths = Object.keys(summary.files).sort();
  const { counts } = summary;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: 12 }}>
      {workItemId && onOpenWorkItem ? (
        <div>
          <button
            type="button"
            onClick={() => onOpenWorkItem(workItemId)}
            style={openTaskButtonStyle}
            data-testid="snapshot-slideover-open-task"
          >
            Open task
          </button>
        </div>
      ) : null}
      <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 10px", color: "var(--text-secondary)", fontSize: 11 }}>
        <span>Created</span>
        <span>{formatAbsolute(summary.snapshot.created_at)}</span>
      </div>
      <div>
        <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--text-secondary)", marginBottom: 4 }}>
          {paths.length} file{paths.length === 1 ? "" : "s"}
          {counts.created > 0 ? <span style={{ marginLeft: 6, color: "var(--severity-ok, #86efac)" }}>+{counts.created}</span> : null}
          {counts.updated > 0 ? <span style={{ marginLeft: 4, color: "var(--severity-warn, #e5a06a)" }}>~{counts.updated}</span> : null}
          {counts.deleted > 0 ? <span style={{ marginLeft: 4, color: "var(--severity-critical, #f87171)" }}>−{counts.deleted}</span> : null}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {paths.map((path) => {
            const row = summary.files[path]!;
            const oversize = row.kind !== "deleted" && row.entry.state === "oversize";
            const canRestore = row.kind !== "deleted" && row.entry.state === "present";
            return (
              <div
                key={path}
                onClick={() => onOpenFileDiff(path)}
                style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, cursor: "pointer" }}
                data-testid={`snapshot-slideover-file-${path}`}
              >
                <span style={{ ...statusBadgeStyle, color: statusColor(row.kind) }}>{statusLabel(row.kind)}</span>
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }} title={path}>
                  {path}
                </span>
                {canRestore ? (
                  <span onClick={(e) => e.stopPropagation()}>
                    <InlineConfirm
                      triggerLabel="Restore"
                      confirmLabel="Restore"
                      triggerStyle={{ fontSize: 10, padding: "2px 6px" }}
                      onConfirm={() => onRestore(path)}
                    />
                  </span>
                ) : null}
                {oversize ? (
                  <span style={oversizeBadgeStyle}>OVERSIZE</span>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function statusLabel(kind: "created" | "updated" | "deleted"): string {
  switch (kind) {
    case "created":
      return "A";
    case "deleted":
      return "D";
    case "updated":
      return "M";
  }
}

function statusColor(kind: "created" | "updated" | "deleted"): string {
  switch (kind) {
    case "created":
      return "var(--severity-ok, #86efac)";
    case "deleted":
      return "var(--severity-critical, #f87171)";
    case "updated":
      return "var(--severity-warn, #e5a06a)";
  }
}

function renderDiffSide(content: string | null, state: "absent" | "present" | "oversize"): string {
  if (content !== null) return content;
  switch (state) {
    case "absent":
      return "// (file not tracked at this snapshot)";
    case "oversize":
      return "// (file too large to snapshot — size/mtime tracked only)";
    case "present":
      return "// (snapshot blob unreadable)";
  }
}

function formatAbsolute(input: string): string {
  if (!input) return "";
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleString();
}

const openTaskButtonStyle: CSSProperties = {
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "4px 12px",
  fontSize: 11,
  cursor: "pointer",
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

const oversizeBadgeStyle: CSSProperties = {
  fontSize: 9,
  fontWeight: 600,
  letterSpacing: 0.4,
  padding: "0 4px",
  border: "1px solid var(--border-subtle)",
  borderRadius: 3,
  color: "var(--text-secondary)",
  flexShrink: 0,
};
