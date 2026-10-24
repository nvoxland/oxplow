/// The open dock panel: it rises from the bar over the page area (absolute,
/// anchored above the bar, so the page does not reflow). A header with the
/// title and ×, a drag handle on the top edge, Escape closes and returns
/// focus to the panel's icon. Only the open panel's body is mounted.

import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";

import type { TabRef } from "../../tabs/tabState.js";
import { dockStore, DOCK_MAX_HEIGHT_MARGIN, DOCK_MIN_HEIGHT, useDockState } from "./dockStore.js";
import { useDockPanels } from "./dockPanels.js";

/** The height the panel draws at: the remembered one within the column. */
export function clampDockHeight(height: number, columnHeight: number): number {
  const max = Math.max(DOCK_MIN_HEIGHT, columnHeight - DOCK_MAX_HEIGHT_MARGIN);
  return Math.max(DOCK_MIN_HEIGHT, Math.min(max, height));
}

export function DockPanel({ streamId, onOpenPage }: { streamId: string | null; onOpenPage(ref: TabRef): void }) {
  const panels = useDockPanels();
  const { open, height } = useDockState();
  const panel = panels.find((p) => p.id === open) ?? null;
  const rootRef = useRef<HTMLDivElement | null>(null);
  const drag = useRef<{ startY: number; base: number } | null>(null);
  const [columnHeight, setColumnHeight] = useState(Number.POSITIVE_INFINITY);

  // The column is the dock's parent's parent (the dock wraps bar and panel).
  useEffect(() => {
    if (!panel) return;
    const measure = () => {
      const column = rootRef.current?.parentElement?.parentElement;
      if (column && column.clientHeight > 0) setColumnHeight(column.clientHeight);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [panel?.id]);

  // Take focus on open, so Escape reaches the panel.
  useEffect(() => {
    if (panel) rootRef.current?.focus();
  }, [panel?.id]);

  if (!panel) return null;
  const drawn = clampDockHeight(height, columnHeight);

  const close = () => {
    const id = panel.id;
    dockStore.close();
    document.querySelector<HTMLElement>(`[data-testid="dock-${id}"]`)?.focus();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    e.preventDefault();
    close();
  };
  const onHandleDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    try { e.currentTarget.setPointerCapture(e.pointerId); } catch {}
    drag.current = { startY: e.clientY, base: drawn };
  };
  const onHandleMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current || e.buttons === 0) return;
    dockStore.setHeight(clampDockHeight(drag.current.base + (drag.current.startY - e.clientY), columnHeight));
  };
  const onHandleUp = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = null;
    try { e.currentTarget.releasePointerCapture(e.pointerId); } catch {}
  };
  const Body = panel.Body;
  return (
    <div
      ref={rootRef}
      role="region"
      aria-label={panel.title}
      tabIndex={-1}
      data-testid="dock-panel"
      onKeyDown={onKeyDown}
      style={{
        position: "absolute",
        left: 0,
        right: 0,
        bottom: "100%",
        height: drawn,
        display: "flex",
        flexDirection: "column",
        background: "var(--bg-1)",
        borderTop: "1px solid var(--border-strong)",
        boxShadow: "0 -6px 20px rgba(0, 0, 0, 0.35)",
        outline: "none",
        zIndex: 50,
      }}
    >
      <div
        data-testid="dock-panel-resize"
        onPointerDown={onHandleDown}
        onPointerMove={onHandleMove}
        onPointerUp={onHandleUp}
        onPointerCancel={onHandleUp}
        style={{ position: "absolute", top: -3, left: 0, right: 0, height: 6, cursor: "row-resize", userSelect: "none" }}
      />
      <div style={{ display: "flex", alignItems: "center", padding: "4px 12px", borderBottom: "1px solid var(--border-subtle)", flexShrink: 0 }}>
        <strong style={{ flex: 1, fontSize: "var(--text-xs)" }}>{panel.title}</strong>
        <button
          type="button"
          data-testid="dock-panel-close"
          aria-label={`Close ${panel.title}`}
          title="Close (Esc)"
          onClick={close}
          style={{ background: "none", border: "none", color: "var(--text-secondary)", cursor: "pointer", fontSize: 16, lineHeight: 1 }}
        >
          ×
        </button>
      </div>
      <div data-testid={`dock-panel-${panel.id}`} style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        <Body streamId={streamId} onOpenPage={onOpenPage} />
      </div>
    </div>
  );
}
