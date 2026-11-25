import { useEffect, useState } from "react";
import { isReplaced, onRemoteConnectionState } from "../tauri-bridge/transport.js";

/** Over everything once another page load of this device took over (the
 *  daemon keeps one per device and project): this window asks nothing more.
 *  Use it here loads the window again, and the new load takes the device
 *  back. `reload` is the page's own; a test passes its stand-in. */
export function ReplacedScreen({ reload = () => window.location.reload() }: { reload?: () => void }) {
  const [shown, setShown] = useState(isReplaced);
  useEffect(
    () =>
      onRemoteConnectionState((state) => {
        if (state === "replaced") setShown(true);
      }),
    [],
  );
  if (!shown) return null;
  return (
    <div
      data-testid="replaced-screen"
      role="alertdialog"
      aria-label="oxplow is open in another tab"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 10_000,
        background: "var(--bg-1)",
        color: "var(--fg)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
        padding: 24,
        textAlign: "center",
      }}
    >
      <div style={{ fontSize: "var(--text-lg)" }}>oxplow is open in another tab</div>
      <div style={{ color: "var(--text-muted)", maxWidth: 420 }}>
        This project is open in another tab or window of this browser. Only one of them can be used at a time.
      </div>
      <button type="button" data-testid="replaced-use-here" onClick={reload} autoFocus>
        Use it here
      </button>
    </div>
  );
}
