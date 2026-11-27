import { useEffect, type MutableRefObject } from "react";
import { onRemoteReconnect } from "../api.js";
import type { TabReport } from "../tauri-bridge/generated/bindings.js";

/** Send the window's last tab report again, with the page runs open now: on
 *  every heartbeat (`everyMs`), so the daemon never forgets an idle window,
 *  and at once after a reconnect, because the daemon forgets a window's
 *  report when its socket closes. Nothing is sent before the first report. */
export function useTabReportRepeats(
  last: MutableRefObject<TabReport | null>,
  reporter: { send(report: TabReport): void },
  runIds: () => string[],
  everyMs: number,
) {
  useEffect(() => {
    const again = () => {
      if (!last.current) return;
      const report = { ...last.current, runs: runIds() };
      last.current = report;
      reporter.send(report);
    };
    const beat = setInterval(again, everyMs);
    const off = onRemoteReconnect(again);
    return () => {
      clearInterval(beat);
      off();
    };
    // `last` and `runIds` are read when it fires, never captured stale.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reporter, everyMs]);
}
