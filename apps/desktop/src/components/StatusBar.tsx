import { BackgroundTaskIndicator } from "./BackgroundTaskIndicator.js";
import { AlertsIndicator } from "./Alerts/AlertsIndicator.js";
import type { TabRef } from "../tabs/tabState.js";

interface Props {
  /** Opens the Alerts page from the bell. */
  onOpenPage(ref: TabRef): void;
}

/**
 * Right-aligned bottom-rail composite: the alerts bell and the
 * background-task indicator (only visible when something's running).
 * Where you are — stream, thread, branch — and search live in the title
 * bar (`TitleBar.tsx`).
 */
export function StatusBar({ onOpenPage }: Props) {
  return (
    <div style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <AlertsIndicator onOpenPage={onOpenPage} />
      <BackgroundTaskIndicator />
    </div>
  );
}
