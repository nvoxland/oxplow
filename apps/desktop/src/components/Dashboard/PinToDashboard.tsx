import { useEffect, useRef, useState, type CSSProperties } from "react";
import { addDashboardItem, createDashboard, listDashboards, type Dashboard } from "../../api.js";
import { customDashboardRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { recordOpError } from "../../opErrors.js";
import { showToast } from "../toastStore.js";

/** What a pin adds: a lens tile or a query tile (P4.7). */
export type PinnedTile =
  | { kind: "lens"; lensId: string; optionsJson?: string }
  | { kind: "query"; sql: string; display: string; optionsJson?: string };

/**
 * "Pin to Dashboard": a button opening the dashboards menu
 * (`PinToDashboardMenu`). A page's nav bar opens the menu from its ☰
 * instead; this is for a page without that bar (a custom page).
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
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement | null>(null);

  // Close on an outside press, like the app's other popovers (the menu
  // closes itself on Escape).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  return (
    <span ref={wrapRef} style={{ position: "relative" }}>
      <button
        type="button"
        data-testid={testId}
        disabled={!!disabledReason}
        title={disabledReason ?? undefined}
        onClick={() => setOpen((v) => !v)}
      >
        Pin to Dashboard
      </button>
      {open ? (
        <PinToDashboardMenu tile={tile} testId={testId} onOpenPage={onOpenPage} onDone={() => setOpen(false)} style={pinMenuStyle} />
      ) : null}
    </span>
  );
}

/**
 * The dashboards (and "New Dashboard…", named inline) to add `tile` to:
 * picking one adds it and opens that dashboard. Loads the dashboards when
 * it opens; `onDone` closes it (picked, or Escape).
 */
export function PinToDashboardMenu({
  tile,
  testId,
  onOpenPage,
  onDone,
  style,
}: {
  tile: PinnedTile;
  /** Test-id prefix: `<testId>-menu`, `-to-<id>`, `-new`, `-new-name`. */
  testId: string;
  onOpenPage(ref: TabRef): void;
  onDone(): void;
  style?: CSSProperties;
}) {
  const [dashboards, setDashboards] = useState<Dashboard[] | null>(null);
  // New Dashboard… asks for its name inline (it once made "My Dashboard"
  // without asking).
  const [naming, setNaming] = useState<string | null>(null);
  const doneRef = useRef(onDone);
  doneRef.current = onDone;

  useEffect(() => {
    let live = true;
    listDashboards()
      .then((list) => {
        if (live) setDashboards(list);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "List dashboards", message: String(e) });
        doneRef.current();
      });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") doneRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      live = false;
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  async function pin(dashboardId: string, title: string) {
    onDone();
    try {
      await addDashboardItem({ dashboardId, ...tile });
      showToast({ message: `Pinned to ${title}.` });
      onOpenPage(customDashboardRef(dashboardId));
    } catch (e) {
      recordOpError({ label: "Pin to dashboard", message: String(e) });
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

  if (!dashboards) return null;
  return (
    <div data-testid={`${testId}-menu`} style={{ display: "flex", flexDirection: "column", ...style }}>
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
