import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import {
  listBackgroundTasks,
  subscribeBackgroundTaskEvents,
  type BackgroundTask,
} from "../api.js";

const KIND_LABEL: Record<BackgroundTask["kind"], string> = {
  git: "Git",
  "code-quality": "Code quality",
  lsp: "LSP",
  "notes-resync": "Notes",
};

/**
 * Bottom-bar widget that shows currently-running long-running ops
 * (git pull/push, code-quality scans, LSP startup, notes resync) and
 * lets the user click to expand a popover listing every live row.
 *
 * Renders nothing when no tasks are running. Uses
 * `subscribeBackgroundTaskEvents` to refetch on every transition; the
 * runtime auto-evicts done/failed rows after a short grace window.
 */
export function BackgroundTaskIndicator() {
  const [tasks, setTasks] = useState<BackgroundTask[]>([]);
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [popoverCoords, setPopoverCoords] = useState<CSSProperties>({});

  useEffect(() => {
    let cancelled = false;
    async function refresh() {
      try {
        const next = await listBackgroundTasks();
        if (!cancelled) setTasks(next);
      } catch {
        // ignore — runtime may be tearing down
      }
    }
    refresh();
    const unsub = subscribeBackgroundTaskEvents(refresh);
    return () => { cancelled = true; unsub(); };
  }, []);

  // Keep elapsed-time labels live without bashing IPC.
  const [, setTick] = useState(0);
  useEffect(() => {
    if (tasks.length === 0) return;
    const id = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, [tasks.length]);

  useEffect(() => {
    if (!open) return;
    function place() {
      const btn = btnRef.current;
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const width = 340;
      const coords: CSSProperties = {
        position: "fixed",
        width,
        bottom: window.innerHeight - rect.top + 4,
        left: Math.max(8, Math.min(window.innerWidth - width - 8, rect.left)),
      };
      setPopoverCoords(coords);
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    function onDocClick(e: MouseEvent) {
      const t = e.target as Node;
      if (popRef.current?.contains(t)) return;
      if (btnRef.current?.contains(t)) return;
      setOpen(false);
    }
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDocClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDocClick);
    };
  }, [open]);

  const running = tasks.filter((t) => t.status === "running");
  if (tasks.length === 0) return null;

  // Pick the "primary" row — the longest-running running task, or the
  // most recent ended one if nothing's currently running.
  const primary = running[0] ?? tasks[tasks.length - 1];
  const summaryLabel = running.length > 1
    ? `${running.length} tasks running`
    : primary.label;

  return (
    <>
      <button
        type="button"
        ref={btnRef}
        data-testid="background-task-indicator"
        title="Background tasks (click to expand)"
        onClick={() => setOpen((v) => !v)}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 8,
          padding: "2px 8px",
          height: 22,
          background: "transparent",
          border: "1px solid var(--border)",
          borderRadius: 4,
          color: "var(--fg)",
          fontSize: 11,
          cursor: "pointer",
          maxWidth: 240,
        }}
      >
        <span style={{
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}>
          {summaryLabel}
        </span>
        <ProgressBar task={primary} width={60} />
      </button>
      {open && (
        <div
          ref={popRef}
          style={{
            ...popoverCoords,
            background: "var(--bg-elevated, #1e1e1e)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            boxShadow: "0 6px 24px rgba(0,0,0,0.4)",
            padding: 8,
            zIndex: 1000,
            maxHeight: 400,
            overflow: "auto",
          }}
        >
          <div style={{
            fontSize: 11,
            color: "var(--muted)",
            padding: "4px 6px 6px",
            borderBottom: "1px solid var(--border)",
            marginBottom: 4,
          }}>
            Background tasks ({running.length} running)
          </div>
          {tasks.length === 0 ? (
            <div style={{ padding: 8, color: "var(--muted)", fontSize: 12 }}>
              No tasks.
            </div>
          ) : (
            tasks.map((task) => (
              <BackgroundTaskRow key={task.id} task={task} />
            ))
          )}
        </div>
      )}
    </>
  );
}

function BackgroundTaskRow({ task }: { task: BackgroundTask }) {
  const elapsed = Math.max(0, Math.round(((task.endedAt ?? Date.now()) - task.startedAt) / 1000));
  return (
    <div
      data-testid={`background-task-row-${task.id}`}
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        padding: "6px 6px",
        borderRadius: 4,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
        <span style={{ color: "var(--muted)", fontSize: 10, minWidth: 80 }}>
          {KIND_LABEL[task.kind]}
        </span>
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {task.label}
        </span>
        <StatusGlyph status={task.status} />
        <span style={{ color: "var(--muted)", fontSize: 10 }}>{elapsed}s</span>
      </div>
      {task.detail && (
        <div style={{ color: "var(--muted)", fontSize: 11, paddingLeft: 86 }}>
          {task.detail}
        </div>
      )}
      {task.error && (
        <div style={{ color: "#fca5a5", fontSize: 11, paddingLeft: 86 }}>
          {task.error}
        </div>
      )}
      <div style={{ paddingLeft: 86 }}>
        <ProgressBar task={task} width={220} />
      </div>
    </div>
  );
}

function ProgressBar({ task, width }: { task: BackgroundTask; width: number }) {
  const isDone = task.status === "done";
  const isFailed = task.status === "failed";
  const indeterminate = task.status === "running" && task.progress === null;
  const filled = isDone ? 1 : (task.progress ?? 0);
  const trackColor = "rgba(255,255,255,0.08)";
  const fillColor = isFailed
    ? "#f87171"
    : isDone
      ? "#4ade80"
      : "var(--accent, #4a9eff)";
  return (
    <div
      style={{
        position: "relative",
        width,
        height: 4,
        borderRadius: 2,
        background: trackColor,
        overflow: "hidden",
        flexShrink: 0,
      }}
    >
      {indeterminate ? (
        <div style={{
          position: "absolute",
          inset: 0,
          width: "33%",
          background: fillColor,
          animation: "oxplow-progress-indeterminate 1.4s ease-in-out infinite",
        }} />
      ) : (
        <div style={{
          width: `${Math.round(filled * 100)}%`,
          height: "100%",
          background: fillColor,
          transition: "width 200ms ease-out",
        }} />
      )}
    </div>
  );
}

function StatusGlyph({ status }: { status: BackgroundTask["status"] }) {
  if (status === "done") return <span style={{ color: "#4ade80" }} aria-label="done">✓</span>;
  if (status === "failed") return <span style={{ color: "#f87171" }} aria-label="failed">✕</span>;
  return <span style={{ color: "var(--muted)" }} aria-label="running">…</span>;
}
