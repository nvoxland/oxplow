/**
 * Trailing-debounce + single-flight wrapper for an expensive async refresh.
 *
 * Extends the `subscribeMetricRefresh` discipline (tsk91/tsk197) with an
 * in-flight guard, which is the part that matters when the refresh can
 * outlast the event cadence that triggers it. Debouncing alone still lets
 * a slow refresh overlap the next one; single-flight bounds concurrency to
 * exactly one regardless of how slow the work is.
 *
 * Contract:
 * - Schedules inside one window collapse into a single run.
 * - At most one run is ever in flight.
 * - A schedule that arrives mid-run queues exactly one follow-up run (not
 *   one per schedule), so the final state is never stale.
 * - A rejected run doesn't wedge the gate.
 */
export type CoalescedRefresh = {
  /** Request a refresh; coalesces with anything already pending. */
  schedule: () => void;
  /** Stop the pending timer and suppress any queued follow-up run. */
  cancel: () => void;
};

export function coalescedRefresh(
  run: () => Promise<unknown>,
  debounceMs = 250,
): CoalescedRefresh {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight = false;
  let queued = false;
  let cancelled = false;

  const start = async () => {
    inFlight = true;
    try {
      await run();
    } catch {
      // A failed scan must not wedge the gate shut — the next event
      // should still be able to refresh.
    }
    inFlight = false;
    // One follow-up for however many events arrived mid-run, so the
    // result reflects the state they signalled rather than going stale.
    if (queued && !cancelled) {
      queued = false;
      void start();
    }
  };

  return {
    schedule: () => {
      if (cancelled) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        if (inFlight) {
          queued = true;
          return;
        }
        void start();
      }, debounceMs);
    },
    cancel: () => {
      cancelled = true;
      queued = false;
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
    },
  };
}
