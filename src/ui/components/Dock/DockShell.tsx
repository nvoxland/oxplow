import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { DockSide, ToolWindow } from "./ToolWindow.js";

interface PersistedState {
  open: boolean;
  size: number;
  activeId: string | null;
}

export interface DockShellProps {
  side: DockSide;
  toolWindows: ToolWindow[];
  /** localStorage key suffix; final key is `oxplow.layout.v1.dock.${storageKey}`. */
  storageKey: string;
  defaultOpen?: boolean;
  /** Width for left/right docks, height for bottom dock. */
  defaultSize?: number;
  minSize?: number;
  maxSize?: number;
  /** Rail behaviour: auto hides the rail when only one tool window is present. */
  railMode?: "auto" | "always" | "never";
  /** Programmatically open and activate a tool window. Token changes retrigger. */
  activateRequest?: { id: string; token: number };
  /** Bottom docks only: extra content rendered on the right side of the rail. */
  railExtra?: ReactNode;
}

const STORAGE_PREFIX = "oxplow.layout.v1.dock.";

export function DockShell({
  side,
  toolWindows,
  storageKey,
  defaultOpen = true,
  defaultSize,
  minSize = 180,
  maxSize = 900,
  railMode = "auto",
  activateRequest,
  railExtra,
}: DockShellProps) {
  const initialSize = defaultSize ?? (side === "bottom" ? 180 : 280);
  const initialActiveId = toolWindows[0]?.id ?? null;

  const [state, setState] = useState<PersistedState>(() =>
    readPersisted(storageKey) ?? { open: defaultOpen, size: initialSize, activeId: initialActiveId },
  );
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    writePersisted(storageKey, state);
  }, [state, storageKey]);

  // If the persisted active tool window was removed in a later build, fall back
  // to the first available one so we don't render an empty panel.
  useEffect(() => {
    if (toolWindows.length === 0) return;
    if (state.activeId && toolWindows.some((tw) => tw.id === state.activeId)) return;
    setState((prev) => ({ ...prev, activeId: toolWindows[0]!.id }));
  }, [toolWindows, state.activeId]);

  useEffect(() => {
    if (!activateRequest) return;
    if (!toolWindows.some((tw) => tw.id === activateRequest.id)) return;
    setState((prev) => ({ ...prev, open: true, activeId: activateRequest.id }));
  }, [activateRequest, toolWindows]);

  const activeTool = useMemo(
    () => toolWindows.find((tw) => tw.id === state.activeId) ?? toolWindows[0] ?? null,
    [toolWindows, state.activeId],
  );

  useEffect(() => {
    if (!dragging) return;
    function stop() {
      setDragging(false);
    }
    function move(event: PointerEvent) {
      setState((prev) => ({ ...prev, size: clampSize(computeSize(side, event), minSize, maxSize) }));
    }
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    document.body.style.cursor = side === "bottom" ? "row-resize" : "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
  }, [dragging, side, minSize, maxSize]);

  const setActiveId = useCallback((id: string) => {
    setState((prev) => {
      // Clicking the active rail entry while the dock is open toggles it closed.
      // Clicking any entry while closed opens the dock on that entry.
      if (!prev.open) return { ...prev, open: true, activeId: id };
      if (prev.activeId === id) return { ...prev, open: false };
      return { ...prev, activeId: id };
    });
  }, []);

  const showRail =
    railMode === "always" || (railMode === "auto" && toolWindows.length > 1);
  const hasRailExtra = side === "bottom" && !!railExtra;

  if (toolWindows.length === 0 && !hasRailExtra) return null;

  const resizeHandle = (
    <div
      role="separator"
      aria-orientation={side === "bottom" ? "horizontal" : "vertical"}
      aria-label={`Resize ${side} dock`}
      onPointerDown={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      style={{
        flexShrink: 0,
        cursor: side === "bottom" ? "row-resize" : "col-resize",
        background: dragging ? "var(--accent)" : "var(--border)",
        transition: dragging ? "none" : "background 120ms ease",
        ...(side === "bottom" ? { height: 6, width: "100%" } : { width: 6, height: "100%" }),
      }}
    />
  );

  const railStyle: React.CSSProperties =
    side === "bottom"
      ? {
          display: "flex",
          gap: 2,
          padding: "2px 6px",
          borderTop: "1px solid var(--border)",
          background: "var(--bg-2)",
          fontSize: 11,
        }
      : {
          display: "flex",
          flexDirection: "column",
          width: 28,
          borderRight: side === "left" ? undefined : "1px solid var(--border)",
          borderLeft: side === "right" ? undefined : "1px solid var(--border)",
          background: "var(--bg-2)",
          padding: "6px 0",
          gap: 2,
          alignItems: "center",
          flexShrink: 0,
        };

  const rail = (showRail || hasRailExtra) ? (
    <div style={{ ...railStyle, ...(side === "bottom" ? { alignItems: "center" } : {}) }}>
      {showRail ? toolWindows.map((tw) => {
        const active = state.open && tw.id === (activeTool?.id ?? "");
        const baseStyle: React.CSSProperties = {
          background: active ? "var(--bg)" : "transparent",
          color: active ? "var(--fg)" : "var(--muted)",
          border: "1px solid",
          borderColor: active ? "var(--border)" : "transparent",
          borderRadius: 4,
          cursor: "pointer",
          fontFamily: "inherit",
          fontSize: 11,
        };
        const orientedStyle: React.CSSProperties =
          side === "bottom"
            ? { ...baseStyle, padding: "2px 8px" }
            : {
                ...baseStyle,
                padding: "6px 0",
                width: 22,
                writingMode: "vertical-rl",
                transform: side === "left" ? "rotate(180deg)" : undefined,
              };
        return (
          <button type="button"
            key={tw.id}
            onClick={() => setActiveId(tw.id)}
            title={tw.label}
            data-testid={`dock-tab-${tw.id}`}
            data-active={active ? "true" : "false"}
            style={orientedStyle}
          >
            {tw.label}
          </button>
        );
      }) : null}
      {hasRailExtra ? (
        <>
          <span style={{ flex: 1 }} />
          <span style={{ display: "inline-flex", alignItems: "center" }}>{railExtra}</span>
        </>
      ) : null}
    </div>
  ) : null;

  // Render every tool window's content always so they keep their internal
  // state (subscriptions, file-tree caches, terminal scroll, etc.) when the
  // dock collapses or the user switches tool windows. Visibility is purely a
  // CSS toggle.
  const content = (
    <div
      style={{
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        overflow: "hidden",
        display: state.open ? "flex" : "none",
        flexDirection: "column",
      }}
    >
      {toolWindows.map((tw) => {
        const isActive = activeTool ? tw.id === activeTool.id : false;
        return (
          <div
            key={tw.id}
            data-testid={`dock-panel-${tw.id}`}
            data-active={isActive ? "true" : "false"}
            style={{
              flex: 1,
              minHeight: 0,
              minWidth: 0,
              display: state.open && isActive ? "flex" : "none",
              flexDirection: "column",
            }}
          >
            {tw.render()}
          </div>
        );
      })}
    </div>
  );

  if (side === "bottom") {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          height: state.open ? state.size : "auto",
          borderTop: "1px solid var(--border)",
        }}
      >
        {state.open ? resizeHandle : null}
        {content}
        {rail}
      </div>
    );
  }

  const bodyStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: side === "left" ? "row" : "row-reverse",
    width: state.open ? state.size : showRail ? 28 : 0,
    height: "100%",
    overflow: "hidden",
  };
  return (
    <div style={{ display: "flex", height: "100%" }}>
      {side === "right" ? (state.open ? resizeHandle : null) : null}
      <div style={bodyStyle}>
        {rail}
        {content}
      </div>
      {side === "left" ? (state.open ? resizeHandle : null) : null}
    </div>
  );
}

function clampSize(next: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, next));
}

function computeSize(side: DockSide, event: PointerEvent): number {
  if (side === "left") return event.clientX;
  if (side === "right") return window.innerWidth - event.clientX;
  return window.innerHeight - event.clientY;
}

function readPersisted(storageKey: string): PersistedState | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + storageKey);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PersistedState>;
    if (typeof parsed.open !== "boolean") return null;
    if (typeof parsed.size !== "number" || !Number.isFinite(parsed.size)) return null;
    return {
      open: parsed.open,
      size: parsed.size,
      activeId: typeof parsed.activeId === "string" ? parsed.activeId : null,
    };
  } catch {
    return null;
  }
}

function writePersisted(storageKey: string, state: PersistedState): void {
  try {
    window.localStorage.setItem(STORAGE_PREFIX + storageKey, JSON.stringify(state));
  } catch {}
}
