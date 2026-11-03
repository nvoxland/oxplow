/// The bottom row of the center column: what the caller puts at the left
/// (the status bar), the dock's icons justified to the right as a tray's,
/// and the open panel rising above the row. The open panel is a popup: a
/// press outside the dock (the panel and its bar) closes it, as a menu
/// closes; a press on the bar is the icon's own (its click toggles).

import { useEffect, useRef, type ReactNode } from "react";

import type { TabRef } from "../../tabs/tabState.js";
import { DockBar } from "./DockBar.js";
import { DockPanel } from "./DockPanel.js";
import { dockStore, useDockState } from "./dockStore.js";

export function Dock({ streamId, onOpenPage, children }: { streamId: string | null; onOpenPage(ref: TabRef): void; children?: ReactNode }) {
  const { open } = useDockState();
  const rootRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) dockStore.close();
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);
  return (
    // Gap above the bottom rail, matching the other panel gaps; the chrome
    // behind shows through.
    <div ref={rootRef} style={{ position: "relative", flexShrink: 0, marginTop: 6 }}>
      <DockPanel streamId={streamId} onOpenPage={onOpenPage} />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 8,
          padding: "4px 10px",
          background: "var(--bg-2)",
          minHeight: 26,
        }}
      >
        <div style={{ display: "inline-flex", alignItems: "center", gap: 8, minWidth: 0 }}>{children}</div>
        <DockBar />
      </div>
    </div>
  );
}
