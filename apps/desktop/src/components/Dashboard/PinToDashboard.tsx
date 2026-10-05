import { useEffect, useRef, useState, type CSSProperties } from "react";
import { addDashboardItem, createDashboard, listDashboards, type Dashboard } from "../../api.js";
import { customDashboardRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { recordOpError } from "../opErrorsStore.js";
import { showToast } from "../toastStore.js";

/** What a pin adds: a lens tile or a query tile (P4.7). */
export type PinnedTile =
  | { kind: "lens"; lensId: string; optionsJson?: string }
  | { kind: "query"; sql: string; display: string; optionsJson?: string };

/**
 * "Pin to Dashboard": a menu of the dashboards (and "New Dashboard…") that
 * adds `tile` to the one picked and opens it. The lens page pins its lens;
 * the explorer pins its query.
 */
export function PinToDashboard({
  tile,
  testId,
  onOpenPage,
  disabledReason = null,
}: {
  /** The tile to add (see `addDashboardItem`). */
  tile: PinnedTile;
  /** Test-id prefix for the button and its menu. */
  testId: string;
  onOpenPage(ref: TabRef): void;
  /** Why it can't be pinned now, shown on the disabled button. */
  disabledReason?: string | null;
}) {
  const [dashboards, setDashboards] = useState<Dashboard[] | null>(null);
  // New Dashboard… asks for its name inline (tsk1045: it made "My
  // Dashboard" without asking).
  const [naming, setNaming] = useState<string | null>(null);
  const wrapRef = useRef<HTMLSpanElement | null>(null);
  const isOpen = dashboards !== null;

  // Close on an outside press or Escape, like the app's other popovers.
  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: PointerEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setDashboards(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setDashboards(null);
        setNaming(null);
      }
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  async function pin(dashboardId: string, title: string) {
    setDashboards(null);
    try {
      await addDashboardItem({ dashboardId, ...tile });
      showToast({ message: `Pinned to ${title}.` });
      onOpenPage(customDashboardRef(dashboardId));
    } catch (e) {
      recordOpError({ label: "Pin to dashboard", message: String(e) });
    }
  }

  async function toggle() {
    if (dashboards) {
      setDashboards(null);
      return;
    }
    try {
      setDashboards(await listDashboards());
    } catch (e) {
      recordOpError({ label: "List dashboards", message: String(e) });
    }
  }

  async function pinToNew(title: string) {
    setNaming(null);
    try {
      const d = await createDashboard(title);
      await pin(d.id, d.title);
    } catch (e) {
      recordOpError({ label: "Create dashboard", message: String(e) });
    }
  }

  return (
    <span ref={wrapRef} style={{ position: "relative" }}>
      <button
        type="button"
        data-testid={testId}
        disabled={!!disabledReason}
        title={disabledReason ?? undefined}
        onClick={() => void toggle()}
      >
        Pin to Dashboard
      </button>
      {dashboards ? (
        <div data-testid={`${testId}-menu`} style={pinMenuStyle}>
          {dashboards.map((d) => (
            <button
              key={d.id}
              type="button"
              data-testid={`${testId}-to-${d.id}`}
              style={pinItemStyle}
              onClick={() => void pin(d.id, d.title)}
            >
              {d.title}
            </button>
          ))}
          {naming === null ? (
            <button type="button" data-testid={`${testId}-new`} style={pinItemStyle} onClick={() => setNaming("")}>
              New Dashboard…
            </button>
          ) : (
            <form
              style={{ display: "flex", gap: 4, padding: 4 }}
              onSubmit={(e) => {
                e.preventDefault();
                if (naming.trim()) void pinToNew(naming.trim());
              }}
            >
              <input
                data-testid={`${testId}-new-name`}
                autoFocus
                placeholder="Dashboard name"
                value={naming}
                onChange={(e) => setNaming(e.target.value)}
              />
              <button type="submit" data-testid={`${testId}-new-create`} disabled={!naming.trim()}>
                Create
              </button>
            </form>
          )}
        </div>
      ) : null}
    </span>
  );
}


const pinMenuStyle: CSSProperties = {
  position: "absolute",
  top: "100%",
  right: 0,
  zIndex: 10,
  marginTop: 4,
  minWidth: 200,
  display: "flex",
  flexDirection: "column",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 4,
};
const pinItemStyle: CSSProperties = {
  background: "none",
  border: "none",
  textAlign: "left",
  padding: "6px 8px",
  font: "inherit",
  color: "var(--text-primary)",
  cursor: "pointer",
};
