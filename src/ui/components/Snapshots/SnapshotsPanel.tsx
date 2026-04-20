import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  getSnapshotFileDiff,
  getSnapshotPairDiff,
  getSnapshotSummary,
  listSnapshots,
  restoreFileFromSnapshot,
  subscribeSnapshotEvents,
  type FileSnapshot,
  type SnapshotSummary,
  type Stream,
} from "../../api.js";
import { logUi } from "../../logger.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { ConfirmDialog } from "../ConfirmDialog.js";

interface Props {
  stream: Stream | null;
  onOpenDiff?(spec: DiffSpec): void;
}

export function SnapshotsPanel({ stream, onOpenDiff }: Props) {
  const [snapshots, setSnapshots] = useState<FileSnapshot[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [summary, setSummary] = useState<SnapshotSummary | null>(null);
  const [pendingRestore, setPendingRestore] = useState<string | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [compareMode, setCompareMode] = useState(false);
  const [compareBaseId, setCompareBaseId] = useState<string | null>(null);
  const [detailWidth, setDetailWidth] = useState(380);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!stream) {
      setSnapshots([]);
      setSelectedId(null);
      return;
    }
    let cancelled = false;
    const load = () => {
      setLoading(true);
      setError(null);
      void listSnapshots(stream.id, 200)
        .then((list) => {
          if (cancelled) return;
          setSnapshots(list);
          setLoading(false);
        })
        .catch((err) => {
          if (cancelled) return;
          logUi("warn", "list snapshots failed", { error: String(err) });
          setError(String(err));
          setLoading(false);
        });
    };
    load();
    const unsubscribe = subscribeSnapshotEvents(stream.id, () => load());
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [stream?.id]);

  useEffect(() => {
    if (!selectedId) {
      setSummary(null);
      return;
    }
    let cancelled = false;
    setSummaryLoading(true);
    void getSnapshotSummary(selectedId)
      .then((result) => {
        if (cancelled) return;
        setSummary(result);
        setSummaryLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "snapshot summary failed", { error: String(err) });
        setSummaryLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  useEffect(() => {
    if (!dragging) return;
    const onMove = (event: PointerEvent) => {
      const container = document.getElementById("snapshots-panel-root");
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

  const handleRowClick = (id: string) => {
    if (compareMode) {
      if (compareBaseId === id) {
        setCompareBaseId(null);
      } else if (!compareBaseId) {
        setCompareBaseId(id);
      } else {
        setSelectedId(id);
      }
      return;
    }
    setSelectedId(id);
  };

  const handleOpenFileDiff = async (path: string) => {
    if (!stream || !onOpenDiff || !selectedId) return;
    try {
      let result;
      let label: string;
      if (compareMode && compareBaseId) {
        result = await getSnapshotPairDiff(compareBaseId, selectedId, path);
        label = `${compareBaseId.slice(-6)} → ${selectedId.slice(-6)}`;
      } else {
        result = await getSnapshotFileDiff(selectedId, path);
        label = "parent → snapshot";
      }
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
  };

  const handleRestore = (path: string) => {
    if (!stream || !selectedId) return;
    setPendingRestore(path);
  };

  const performRestore = async (path: string) => {
    if (!stream || !selectedId) return;
    try {
      await restoreFileFromSnapshot(stream.id, selectedId, path);
    } catch (err) {
      logUi("warn", "restore snapshot file failed", { error: String(err) });
      window.alert(`Restore failed: ${String(err)}`);
    }
  };

  const filterLower = filter.trim().toLowerCase();
  const filteredSnapshots = filterLower
    ? snapshots.filter((snap) => {
        const description = snap.turn_id
          ? (snap.turn_prompt ?? `turn ${snap.turn_id.slice(-6)}`)
          : "External";
        return description.toLowerCase().includes(filterLower);
      })
    : snapshots;

  return (
    <div id="snapshots-panel-root" style={containerStyle}>
      <div style={{ display: "flex", flex: 1, minHeight: 0, minWidth: 0 }}>
        <div style={leftPaneStyle}>
          <div style={toolbarStyle}>
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter snapshots"
              style={{ ...inputStyle, flex: 1, minWidth: 120 }}
            />
            <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11, color: "var(--muted)" }}>
              <input
                type="checkbox"
                checked={compareMode}
                onChange={(e) => {
                  setCompareMode(e.target.checked);
                  setCompareBaseId(null);
                }}
              />
              Compare mode
            </label>
            {compareMode ? (
              <span style={{ fontSize: 11, color: "var(--muted)" }}>
                {compareBaseId
                  ? selectedId
                    ? `base: ${compareBaseId.slice(-6)} → current: ${selectedId.slice(-6)}`
                    : `base: ${compareBaseId.slice(-6)} — click another row to compare`
                  : "click a row to pick the base"}
              </span>
            ) : null}
            <div style={{ marginLeft: "auto", fontSize: 11, color: "var(--muted)" }}>
              {loading ? "loading…" : filterLower ? `${filteredSnapshots.length} / ${snapshots.length}` : `${snapshots.length}`}
            </div>
          </div>
          <div style={listStyle}>
            {error ? (
              <div style={{ padding: 12, color: "#ff6b6b", fontSize: 12 }}>{error}</div>
            ) : !stream ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No stream selected.</div>
            ) : snapshots.length === 0 ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No snapshots yet.</div>
            ) : filteredSnapshots.length === 0 ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No snapshots match filter.</div>
            ) : (
              filteredSnapshots.map((snap) => (
                <SnapshotRow
                  key={snap.id}
                  snap={snap}
                  selected={selectedId === snap.id}
                  compareBase={compareBaseId === snap.id}
                  onClick={() => handleRowClick(snap.id)}
                />
              ))
            )}
          </div>
        </div>
        <div
          onPointerDown={(e) => {
            e.preventDefault();
            setDragging(true);
          }}
          style={{ ...resizeHandleStyle, background: dragging ? "var(--accent)" : undefined }}
        />
        <div style={{ ...detailPaneStyle, width: detailWidth }}>
          <DetailPane
            summary={summary}
            loading={summaryLoading}
            onOpenFileDiff={handleOpenFileDiff}
            onRestore={handleRestore}
          />
        </div>
      </div>
      {pendingRestore ? (
        <ConfirmDialog
          message={`Restore ${pendingRestore} to its content from this snapshot? This overwrites the current file in the worktree.`}
          confirmLabel="Restore"
          destructive
          onConfirm={() => {
            const path = pendingRestore;
            setPendingRestore(null);
            void performRestore(path);
          }}
          onCancel={() => setPendingRestore(null)}
        />
      ) : null}
    </div>
  );
}

function SnapshotRow({
  snap,
  selected,
  compareBase,
  onClick,
}: {
  snap: FileSnapshot;
  selected: boolean;
  compareBase: boolean;
  onClick(): void;
}) {
  const date = formatRelative(snap.created_at);
  const isAgent = snap.kind === "turn-end" && snap.turn_id != null;
  const description = isAgent
    ? (snap.turn_prompt ?? "(agent turn)").replace(/\n/g, " ")
    : "External";
  return (
    <div
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        height: 24,
        cursor: "pointer",
        background: selected
          ? "rgba(74, 158, 255, 0.18)"
          : compareBase
            ? "rgba(134, 239, 172, 0.15)"
            : "transparent",
        padding: "0 8px",
        fontSize: 12,
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      {isAgent ? (
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" style={{ flexShrink: 0 }}>
          <path d="M7 1L8.5 5.5H13L9.25 8.25L10.75 13L7 10.25L3.25 13L4.75 8.25L1 5.5H5.5L7 1Z" fill="var(--accent)" />
        </svg>
      ) : (
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" style={{ flexShrink: 0 }}>
          <path d="M9.5 2.5L11.5 4.5L5 11L2 12L3 9L9.5 2.5Z" stroke="var(--muted)" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )}
      <span
        title={description}
        style={{
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          color: isAgent ? "inherit" : "var(--muted)",
        }}
      >
        {description}
      </span>
      <span style={{ color: "var(--muted)", fontSize: 11, minWidth: 90, textAlign: "right", flexShrink: 0 }}>{date}</span>
    </div>
  );
}

function DetailPane({
  summary,
  loading,
  onOpenFileDiff,
  onRestore,
}: {
  summary: SnapshotSummary | null;
  loading: boolean;
  onOpenFileDiff(path: string): void;
  onRestore(path: string): void;
}) {
  if (loading && !summary) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Loading…</div>;
  }
  if (!summary) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>Select a snapshot to see files.</div>;
  }
  const paths = Object.keys(summary.files).sort();
  const { counts } = summary;
  return (
    <div style={{ padding: 10, display: "flex", flexDirection: "column", gap: 10, fontSize: 12, overflow: "auto", height: "100%" }}>
      <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 10px", color: "var(--muted)", fontSize: 11 }}>
        <span>Created</span>
        <span>{formatAbsolute(summary.snapshot.created_at)}</span>
        {summary.snapshot.turn_id ? (
          <>
            <span style={{ alignSelf: "start" }}>Prompt</span>
            <span style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
              {summary.snapshot.turn_prompt ?? summary.snapshot.turn_id}
            </span>
          </>
        ) : null}
      </div>
      <div>
        <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)", marginBottom: 4 }}>
          {paths.length} file{paths.length === 1 ? "" : "s"}
          {counts.created > 0 ? <span style={{ marginLeft: 6, color: "#86efac" }}>+{counts.created}</span> : null}
          {counts.updated > 0 ? <span style={{ marginLeft: 4, color: "#e5a06a" }}>~{counts.updated}</span> : null}
          {counts.deleted > 0 ? <span style={{ marginLeft: 4, color: "#f87171" }}>−{counts.deleted}</span> : null}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {paths.map((path) => {
            const row = summary.files[path]!;
            const oversize = row.entry.state === "oversize";
            const canRestore = row.entry.state === "present";
            const hint = oversize
              ? `Oversize (${formatBytes(row.entry.size)}) — no content diff available.`
              : "Click to open diff. Right-click to restore this version.";
            return (
              <div
                key={path}
                onClick={() => onOpenFileDiff(path)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (!canRestore) return;
                  onRestore(path);
                }}
                title={hint}
                style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, cursor: "pointer" }}
              >
                <span style={{ ...statusBadgeStyle, color: statusColor(row.kind) }}>{statusLabel(row.kind)}</span>
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }} title={path}>
                  {path}
                </span>
                {oversize ? (
                  <span
                    title="Too large to blob. Size/mtime tracked only."
                    style={{
                      fontSize: 9,
                      fontWeight: 600,
                      letterSpacing: 0.4,
                      padding: "0 4px",
                      border: "1px solid var(--border)",
                      borderRadius: 3,
                      color: "var(--muted)",
                      flexShrink: 0,
                    }}
                  >
                    OVERSIZE {formatBytes(row.entry.size)}
                  </span>
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
      return "#86efac";
    case "deleted":
      return "#f87171";
    case "updated":
      return "#e5a06a";
  }
}


function renderDiffSide(
  content: string | null,
  state: "absent" | "present" | "deleted" | "oversize",
): string {
  if (content !== null) return content;
  switch (state) {
    case "absent":
      return "// (file not tracked at this snapshot)";
    case "deleted":
      return "// (file did not exist at this snapshot)";
    case "oversize":
      return "// (file too large to snapshot — size/mtime tracked only)";
    case "present":
      return "// (snapshot blob unreadable)";
  }
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
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

const inputStyle: CSSProperties = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  font: "inherit",
  padding: "3px 6px",
  fontSize: 12,
};

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
  gap: 10,
  padding: 6,
  borderBottom: "1px solid var(--border)",
  flexWrap: "wrap",
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
