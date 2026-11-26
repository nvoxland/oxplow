import { useEffect } from "react";
import { reportUse } from "../tauri-bridge/transport.js";

/** How often, at most, the window tells the daemon it is in use. */
export const USE_REPORT_MS = 1_000;

/** Tell the daemon while the person uses this window (a press, a key,
 *  terminal input — xterm's keys reach the window too), at most once every
 *  `everyMs`: an agent's call with no device of its own goes to the device
 *  used last (`ClientDevices::last_used`). `report` and `now` are a test's. */
export function useReportUse(
  report: () => void = reportUse,
  everyMs: number = USE_REPORT_MS,
  now: () => number = () => Date.now(),
) {
  useEffect(() => {
    let last = -Infinity;
    const used = () => {
      const at = now();
      if (at - last < everyMs) return;
      last = at;
      report();
    };
    window.addEventListener("pointerdown", used, true);
    window.addEventListener("keydown", used, true);
    return () => {
      window.removeEventListener("pointerdown", used, true);
      window.removeEventListener("keydown", used, true);
    };
  }, [report, everyMs, now]);
}
