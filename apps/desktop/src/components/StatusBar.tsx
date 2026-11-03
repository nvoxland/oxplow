import { BackgroundTaskIndicator } from "./BackgroundTaskIndicator.js";

/**
 * Left-aligned bottom-bar content: the background-task indicator (only
 * visible when something's running). The dock's icons (Alerts and the
 * other panels) sit at the right of the same row (`Dock/Dock.tsx`). Where
 * you are — stream, thread, branch — and search live in the title bar
 * (`TitleBar.tsx`).
 */
export function StatusBar() {
  return (
    <div style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <BackgroundTaskIndicator />
    </div>
  );
}
