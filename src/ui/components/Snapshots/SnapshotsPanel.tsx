import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import {
  getEffortFiles,
  getSnapshotPairDiff,
  getSnapshotSummary,
  listEffortsEndingAtSnapshots,
  listSnapshots,
  restoreFileFromSnapshot,
  subscribeSnapshotEvents,
  updateWorkItem,
  type FileSnapshot,
  type SnapshotSummary,
  type Stream,
  type WorkItemStatus,
  type WorkItemPriority,
} from "../../api.js";
import { logUi } from "../../logger.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { InlineConfirm } from "../InlineConfirm.js";
import { InlineStatusPicker } from "../Plan/WorkGroupList.js";

interface Props {
  stream: Stream | null;
  onOpenDiff?(spec: DiffSpec): void;
  /** When set, the panel selects the snapshot with the matching id. Change
   *  the token to request a new selection even if the id repeats. */
  revealSnapshotId?: { snapshotId: string; token: number } | null;
  /** Open the given work item in the edit modal (switching tool windows). */
  onRequestEditWorkItem?(itemId: string): void;
}

export function SnapshotsPanel({ stream, onOpenDiff, revealSnapshotId, onRequestEditWorkItem }: Props) {
  const [snapshots, setSnapshots] = useState<FileSnapshot[]>([]);
  const [effortsBySnapshot, setEffortsBySnapshot] = useState<
    Record<string, Array<{ effortId: string; workItemId: string; threadId: string; title: string; status: WorkItemStatus; priority: WorkItemPriority }>>
  >({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  // A "selection" is either a raw snapshot (no effort) or a specific effort
  // row within a snapshot. `selectedEffortId` disambiguates; when null the
  // summary comes from the raw pair-diff against the previous snapshot.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedEffortId, setSelectedEffortId] = useState<string | null>(null);
  const [summary, setSummary] = useState<SnapshotSummary | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [compareMode, setCompareMode] = useState(false);
  const [compareBaseId, setCompareBaseId] = useState<string | null>(null);
  const [detailWidth, setDetailWidth] = useState(380);
  const [dragging, setDragging] = useState(false);
  // Per-row refs so the reveal-from-elsewhere flow (e.g. "In history" on
  // an Effort row) can scrollIntoView the selected snapshot. Keyed on
  // `${snapshotId}:${effortId ?? ""}` to match the SnapshotRow key used
  // by the list.
  const rowRefs = useRef(new Map<string, HTMLDivElement | null>());
  // Token-gated flash highlight: when a reveal fires, bump this and the
  // matching row paints a brighter background for ~1s so the user's eye
  // lands on it. State-based (not CSS animation) because the reveal can
  // hit the same snapshot twice and we want each reveal to flash again.
  const [flashKey, setFlashKey] = useState<{ key: string; token: number } | null>(null);

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
        .then(async (list) => {
          if (cancelled) return;
          setSnapshots(list);
          try {
            const efforts = await listEffortsEndingAtSnapshots(list.map((s) => s.id));
            if (!cancelled) setEffortsBySnapshot(efforts);
          } catch (err) {
            logUi("warn", "list efforts-by-snapshot failed", { error: String(err) });
          }
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
    const fetcher = selectedEffortId
      ? getEffortFiles(selectedEffortId)
      : getSnapshotSummary(selectedId);
    void fetcher
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
  }, [selectedId, selectedEffortId]);

  useEffect(() => {
    if (!revealSnapshotId) return;
    const target = snapshots.find((s) => s.id === revealSnapshotId.snapshotId);
    if (!target) return;
    setSelectedId(target.id);
    setSelectedEffortId(null);
    // Scroll + flash on the next frame so the DOM has the row rendered
    // from the selection state change above. Key lookup uses the same
    // `${snapshotId}:${effortId ?? ""}` shape SnapshotRow registers
    // under — effortId is null here (reveal target is the snapshot).
    const key = `${target.id}:`;
    const flashToken = revealSnapshotId.token;
    requestAnimationFrame(() => {
      const node = rowRefs.current.get(key);
      if (node) node.scrollIntoView({ block: "nearest", behavior: "smooth" });
      setFlashKey({ key, token: flashToken });
    });
  }, [revealSnapshotId?.snapshotId, revealSnapshotId?.token, snapshots]);

  // Clear the flash highlight after ~1.2s so a stale token doesn't keep
  // brightening the row forever.
  useEffect(() => {
    if (!flashKey) return;
    const timer = setTimeout(() => setFlashKey(null), 1200);
    return () => clearTimeout(timer);
  }, [flashKey?.token]);

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

  const handleChangeStatus = async (
    workItemId: string,
    threadId: string,
    status: WorkItemStatus,
  ) => {
    if (!stream || !threadId) return;
    try {
      await updateWorkItem(stream.id, threadId, workItemId, { status });
      // Optimistic local update; the snapshot-event subscription refreshes.
      setEffortsBySnapshot((prev) => {
        const next: typeof prev = {};
        for (const [sid, list] of Object.entries(prev)) {
          next[sid] = list.map((e) =>
            e.workItemId === workItemId ? { ...e, status } : e,
          );
        }
        return next;
      });
    } catch (err) {
      logUi("warn", "update work item status failed", { error: String(err) });
    }
  };

  const handleRowClick = (id: string, effortId: string | null) => {
    if (compareMode) {
      if (compareBaseId === id) {
        setCompareBaseId(null);
      } else if (!compareBaseId) {
        setCompareBaseId(id);
      } else {
        setSelectedId(id);
        setSelectedEffortId(effortId);
      }
      return;
    }
    setSelectedId(id);
    setSelectedEffortId(effortId);
  };

  const handleOpenFileDiff = async (path: string) => {
    if (!stream || !onOpenDiff || !selectedId || !summary) return;
    try {
      const baseId = compareMode && compareBaseId ? compareBaseId : summary.previousSnapshotId;
      const result = await getSnapshotPairDiff(baseId, selectedId, path);
      const label = baseId
        ? `${baseId.slice(-6)} → ${selectedId.slice(-6)}`
        : `initial → ${selectedId.slice(-6)}`;
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

  const performRestore = async (path: string) => {
    if (!stream || !selectedId) return;
    try {
      await restoreFileFromSnapshot(stream.id, selectedId, path);
    } catch (err) {
      logUi("warn", "restore snapshot file failed", { error: String(err) });
      window.alert(`Restore failed: ${String(err)}`);
    }
  };

  // Snapshots are newest-first. For each `-end` snapshot, a matching `-start`
  // of the same family appears *later* in the array (since it happened
  // earlier in time). If no such `-start` exists, the end is orphaned — the
  // file changed outside a task/turn, so we relabel it "External Change".
  const orphanEndIds = new Set<string>();
  for (let i = 0; i < snapshots.length; i++) {
    const snap = snapshots[i]!;
    const family = endFamily(snap.source);
    if (!family) continue;
    const startSource = family === "task" ? "task-start" : "turn-start";
    let found = false;
    for (let j = i + 1; j < snapshots.length; j++) {
      if (snapshots[j]!.source === startSource) {
        found = true;
        break;
      }
    }
    if (!found) orphanEndIds.add(snap.id);
  }

  const labelFor = (snap: FileSnapshot): string =>
    orphanEndIds.has(snap.id) ? "[ External change ]" : snapshotLabel(snap);

  // Hide `-start` rows: they're only shown via their paired `-end`.
  const visibleSnapshots = snapshots.filter(
    (snap) => snap.source !== "task-start",
  );

  // Synthesize one row per effort when multiple efforts end at the same
  // snapshot (per the per-effort write log attribution model). When zero
  // efforts end at a snapshot, a single "external change" / source-derived
  // row is rendered. When one effort ends at it, a single row with the
  // work item title is rendered (effortId carried so the detail pane calls
  // `getEffortFiles` — same result as the raw pair-diff in the 1-effort
  // fallback, but keeps the UI rendering rule uniform).
  type Row = {
    key: string;
    snap: FileSnapshot;
    label: string;
    effortId: string | null;
    workItemId: string | null;
    threadId: string | null;
    status: WorkItemStatus | null;
    isExternal: boolean;
  };
  const rows: Row[] = [];
  for (const snap of visibleSnapshots) {
    const efforts = effortsBySnapshot[snap.id] ?? [];
    if (efforts.length === 0) {
      rows.push({
        key: snap.id,
        snap,
        label: labelFor(snap),
        effortId: null,
        workItemId: null,
        threadId: null,
        status: null,
        isExternal: orphanEndIds.has(snap.id),
      });
      continue;
    }
    for (const e of efforts) {
      rows.push({
        key: `${snap.id}:${e.effortId}`,
        snap,
        label: e.title,
        effortId: e.effortId,
        workItemId: e.workItemId,
        threadId: e.threadId,
        status: e.status,
        isExternal: false,
      });
    }
  }

  const filterLower = filter.trim().toLowerCase();
  const filteredRows = filterLower
    ? rows.filter((row) => row.label.toLowerCase().includes(filterLower))
    : rows;

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
              {loading ? "loading…" : filterLower ? `${filteredRows.length} / ${rows.length}` : `${rows.length}`}
            </div>
          </div>
          <div style={listStyle}>
            {error ? (
              <div style={{ padding: 12, color: "#ff6b6b", fontSize: 12 }}>{error}</div>
            ) : !stream ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No stream selected.</div>
            ) : rows.length === 0 ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No snapshots yet.</div>
            ) : filteredRows.length === 0 ? (
              <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>No snapshots match filter.</div>
            ) : (
              filteredRows.map((row) => {
                const refKey = `${row.snap.id}:${row.effortId ?? ""}`;
                return (
                  <SnapshotRow
                    key={row.key}
                    snap={row.snap}
                    label={row.label}
                    selected={selectedId === row.snap.id && selectedEffortId === row.effortId}
                    flashing={flashKey?.key === refKey}
                    compareBase={compareBaseId === row.snap.id}
                    onClick={() => handleRowClick(row.snap.id, row.effortId)}
                    status={row.status}
                    workItemId={row.workItemId}
                    isExternal={row.isExternal}
                    rowRef={(node) => { rowRefs.current.set(refKey, node); }}
                    onChangeStatus={
                      row.workItemId && row.threadId
                        ? (nextStatus) => { void handleChangeStatus(row.workItemId!, row.threadId!, nextStatus); }
                        : undefined
                    }
                    onDoubleClick={
                      row.workItemId && onRequestEditWorkItem
                        ? () => onRequestEditWorkItem(row.workItemId!)
                        : undefined
                    }
                  />
                );
              })
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
            onRestore={(path) => { void performRestore(path); }}
            workItemId={
              selectedId && selectedEffortId
                ? (effortsBySnapshot[selectedId] ?? []).find((e) => e.effortId === selectedEffortId)?.workItemId ?? null
                : null
            }
            onOpenWorkItem={onRequestEditWorkItem}
          />
        </div>
      </div>
    </div>
  );
}

function snapshotLabel(snap: FileSnapshot): string {
  if (snap.label) return snap.label;
  switch (snap.source) {
    case "task-start":
      return "Task started";
    case "task-end":
      return "Task ended";
    case "task-event":
      // Task created or status changed (non-in_progress transition).
      // Gap-gated like task-end so back-to-back events don't pile up.
      return "Task update";
    case "startup":
      // Startup snapshots capture changes that happened while the app
      // was down — the source is "startup" but semantically these are
      // external (non-agent) edits to the worktree.
      return "External changes";
    default:
      return "Snapshot";
  }
}

function endFamily(source: string): "task" | "turn" | null {
  if (source === "task-end") return "task";
  return null;
}

function snapshotIconKind(snap: FileSnapshot): "task" | "turn" | "system" {
  if (snap.label_kind) return snap.label_kind;
  switch (snap.source) {
    case "task-start":
    case "task-end":
      return "task";
    default:
      return "system";
  }
}

function SnapshotRow({
  snap,
  label,
  selected,
  flashing,
  compareBase,
  onClick,
  status,
  workItemId,
  isExternal,
  onChangeStatus,
  onDoubleClick,
  rowRef,
}: {
  snap: FileSnapshot;
  label: string;
  selected: boolean;
  flashing: boolean;
  compareBase: boolean;
  onClick(): void;
  status: WorkItemStatus | null;
  workItemId: string | null;
  isExternal: boolean;
  onChangeStatus?(nextStatus: WorkItemStatus): void;
  onDoubleClick?(): void;
  rowRef?(node: HTMLDivElement | null): void;
}) {
  const date = formatRelative(snap.created_at);
  const iconKind = snapshotIconKind(snap);
  const isAgent = iconKind !== "system";
  const description = label;
  // Row "modes":
  //   - external  → distinct glyph, non-interactive status, no dbl-click
  //   - effort    → InlineStatusPicker bound to the work item status
  //   - other     → keep the legacy pencil icon (system rows, etc.)
  const isEffortRow = !!workItemId && !isExternal && !!status;
  return (
    <div
      ref={rowRef}
      onClick={onClick}
      onDoubleClick={isExternal ? undefined : onDoubleClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        height: 36,
        cursor: "pointer",
        // Flashing (reveal) > selected > compareBase > default.
        // Flashing uses a bright accent bg + left stripe so the user's
        // eye locks onto the revealed row when jumping from another
        // surface (e.g. "In history" on an Effort). Selection alone
        // keeps the subtler tint used during normal browsing.
        background: flashing
          ? "rgba(74, 158, 255, 0.40)"
          : selected
            ? "rgba(74, 158, 255, 0.22)"
            : compareBase
              ? "rgba(134, 239, 172, 0.15)"
              : "transparent",
        boxShadow: flashing
          ? "inset 3px 0 0 var(--accent)"
          : selected
            ? "inset 2px 0 0 var(--accent)"
            : undefined,
        transition: "background 0.2s ease-out",
        padding: "0 12px",
        fontSize: 13,
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      {isExternal ? (
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          style={{ flexShrink: 0 }}
          aria-label="External change"
        >
          <title>External change</title>
          {/* Box */}
          <path d="M2 5V12H9V8" stroke="var(--muted)" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
          {/* Arrow escaping up-right */}
          <path d="M7 7L12 2" stroke="var(--muted)" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
          <path d="M8 2H12V6" stroke="var(--muted)" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      ) : isEffortRow && status ? (
        <InlineStatusPicker
          status={status}
          onChange={(next) => onChangeStatus?.(next)}
          locked={!onChangeStatus}
        />
      ) : isAgent ? (
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
  workItemId,
  onOpenWorkItem,
}: {
  summary: SnapshotSummary | null;
  loading: boolean;
  onOpenFileDiff(path: string): void;
  onRestore(path: string): void;
  workItemId: string | null;
  onOpenWorkItem?(itemId: string): void;
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
      {workItemId && onOpenWorkItem ? (
        <div>
          <button
            type="button"
            onClick={() => onOpenWorkItem(workItemId)}
            style={openTaskButtonStyle}
            title="Open this task in the edit modal"
          >
            Open task
          </button>
        </div>
      ) : null}
      <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 10px", color: "var(--muted)", fontSize: 11 }}>
        <span>Created</span>
        <span>{formatAbsolute(summary.snapshot.created_at)}</span>
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
            // Deleted rows have a placeholder entry (the file isn't in the
            // current snapshot) — treat them as non-oversize and non-
            // restorable regardless of the placeholder's state field.
            const oversize = row.kind !== "deleted" && row.entry.state === "oversize";
            const canRestore = row.kind !== "deleted" && row.entry.state === "present";
            const hint = oversize
              ? `Oversize (${formatBytes(row.entry.size)}) — no content diff available.`
              : "Click to open diff. Use the Restore button to restore this version.";
            return (
              <div
                key={path}
                onClick={() => onOpenFileDiff(path)}
                title={hint}
                style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, cursor: "pointer" }}
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
  state: "absent" | "present" | "oversize",
): string {
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
  background: "var(--bg)",
  minHeight: 0,
  display: "flex",
  flexDirection: "column",
  overflow: "hidden",
};

const openTaskButtonStyle: CSSProperties = {
  background: "var(--panel)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 3,
  padding: "3px 10px",
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
